use {
    anyhow::{Context, Result},
    moonmail_core::{IndexChange, SearchIndex},
    moonmail_search::SqliteIndex,
    moonmail_store::Archive,
    std::path::{Path, PathBuf},
};

pub fn root() -> Result<PathBuf> {
    if let Some(root) = std::env::var_os("MOONMAIL_DATA_DIR") {
        return Ok(root.into());
    }
    let home =
        PathBuf::from(std::env::var_os("HOME").context("HOME is unset; set MOONMAIL_DATA_DIR")?);
    let name = if std::env::var_os("MOONMAIL_DEMO").is_some() {
        "moonmail-demo"
    } else {
        "moonmail"
    };
    Ok(home.join("Library/Application Support").join(name))
}

pub fn open_index(root: &Path, archive: &Archive) -> Result<SqliteIndex> {
    let path = root.join("search.sqlite3");
    if !path.exists() {
        archive.reset_index()?;
    }
    Ok(SqliteIndex::open(path)?)
}

/// Commit the projection before acknowledging its durable source journal.
pub fn index_batch(
    archive: &Archive,
    index: &mut impl SearchIndex<Error = moonmail_search::Error>,
) -> Result<bool> {
    let work = archive.index_work(100)?;
    let Some(last) = work.last() else {
        return Ok(false);
    };
    index.apply(
        &work
            .iter()
            .map(|entry| IndexChange::Upsert(entry.id, &entry.document))
            .collect::<Vec<_>>(),
    )?;
    archive.acknowledge_index(last.sequence)?;
    Ok(true)
}

pub fn index_all(root: &Path) -> Result<()> {
    let archive = Archive::open(root)?;
    let mut index = open_index(root, &archive)?;
    while index_batch(&archive, &mut index)? {}
    Ok(())
}
