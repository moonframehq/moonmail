use {
    anyhow::{Context, Result},
    moonmail_core::{BlobStore, MessageId},
    moonmail_store::{Archive, FileBlobs},
    std::{
        collections::BTreeSet,
        io::Write,
        path::{Path, PathBuf},
    },
};

/// Publish complete bytes under a new name. A destination file or symlink is
/// never replaced. Temporary bytes inherit the private tempfile permissions.
pub(crate) fn save_new(path: &Path, bytes: &[u8]) -> Result<()> {
    anyhow::ensure!(path.is_absolute(), "Choose an absolute export destination");
    let parent = path.parent().context("Destination has no parent")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist_noclobber(path)
        .context("Choose a new filename; existing destinations are not overwritten")?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Original, hash-verified RFC message bytes: no MIME reserialization, flag
/// changes, header rewriting, or dependence on the rebuildable search index.
pub(crate) fn message(
    archive: &Archive,
    blobs: &FileBlobs,
    id: MessageId,
    path: &Path,
) -> Result<()> {
    let raw = blobs.get(&archive.message_blob(id)?)?;
    save_new(path, &raw)
}

/// Export an explicit bounded selection into a new private directory. A completed
/// manifest is published last. An ordinary error removes this export's staging
/// directory; an interrupted process may leave one without a manifest.
pub(crate) fn messages(
    archive: &Archive,
    blobs: &FileBlobs,
    ids: &[MessageId],
    parent: &Path,
) -> Result<PathBuf> {
    anyhow::ensure!(
        parent.is_absolute() && parent.is_dir(),
        "Choose an existing absolute export folder"
    );
    anyhow::ensure!(
        !ids.is_empty() && ids.len() <= 100,
        "Choose between one and 100 messages to export"
    );
    let ids: BTreeSet<_> = ids.iter().copied().collect();
    // Freeze all immutable blob references before writing anything. No body is
    // loaded by this metadata pass; export then holds at most one body at a time.
    let sources = ids
        .into_iter()
        .map(|id| Ok((id, archive.message_blob(id)?)))
        .collect::<Result<Vec<_>>>()?;
    let directory = tempfile::Builder::new()
        .prefix("Moonmail Export-")
        .tempdir_in(parent)?;
    let mut entries = Vec::new();
    for (id, blob) in sources {
        let raw = blobs.get(&blob)?;
        let filename = format!("message-{}.eml", id.0);
        save_new(&directory.path().join(&filename), &raw)?;
        entries
            .push(serde_json::json!({"id":id.0,"file":filename,"sha256":blob,"bytes":raw.len()}));
    }
    save_new(
        &directory.path().join("manifest.json"),
        &serde_json::to_vec_pretty(
            &serde_json::json!({"format":"moonmail-eml-export","version":1,"messages":entries}),
        )?,
    )?;
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(directory.keep())
}
