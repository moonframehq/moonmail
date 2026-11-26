use {
    super::{Entry, fingerprint},
    anyhow::{Context, Result, ensure},
    std::{
        fs::{self, File},
        io::Read,
    },
};
pub(super) fn read_delivery(entry: &Entry) -> Result<(Vec<u8>, i64)> {
    let file = File::from(rustix::fs::open(
        &entry.path,
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK,
        rustix::fs::Mode::empty(),
    )?);
    let meta = file.metadata()?;
    ensure!(
        meta.is_file() && fingerprint(&meta) == entry.fingerprint && meta.len() <= 64 * 1024 * 1024,
        "Maildir file changed or exceeds 64 MiB"
    );
    let mut raw = Vec::new();
    (&file).take(64 * 1024 * 1024 + 1).read_to_end(&mut raw)?;
    ensure!(
        fingerprint(&file.metadata()?) == entry.fingerprint
            && fingerprint(&fs::symlink_metadata(&entry.path)?) == entry.fingerprint,
        "Maildir delivery changed during read"
    );
    Ok((raw, std::os::unix::fs::MetadataExt::mtime(&meta)))
}

pub(super) fn write_flags(entry: &Entry, (seen, star): (bool, bool)) -> Result<()> {
    let mut flags: Vec<_> = entry
        .flags
        .chars()
        .filter(|c| !matches!(c, 'S' | 'F'))
        .collect();
    if seen {
        flags.push('S');
    }
    if star {
        flags.push('F');
    }
    flags.sort_unstable();
    let parent = entry.path.parent().context("Invalid Maildir path")?;
    let mailbox = parent.parent().context("Invalid Maildir parent")?;
    let open_dir =
        rustix::fs::OFlags::RDONLY | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::NOFOLLOW;
    let mode = rustix::fs::Mode::empty();
    let folder = rustix::fs::open(mailbox, open_dir, mode)?;
    let source = rustix::fs::openat(
        &folder,
        parent.file_name().context("Invalid Maildir subdirectory")?,
        open_dir,
        mode,
    )?;
    let cur = rustix::fs::openat(&folder, "cur", open_dir, mode)?;
    let name = format!(
        "{}:2,{}{}",
        entry.stem,
        flags.into_iter().collect::<String>(),
        entry.extension
    );
    ensure!(
        fingerprint(&fs::symlink_metadata(&entry.path)?) == entry.fingerprint,
        "Maildir source changed before flag rename"
    );
    rustix::fs::renameat_with(
        &source,
        entry.path.file_name().context("Invalid Maildir filename")?,
        &cur,
        name,
        rustix::fs::RenameFlags::NOREPLACE,
    )?;
    rustix::fs::fsync(&cur)?;
    if parent.ends_with("new") {
        rustix::fs::fsync(&source)?;
    }
    Ok(())
}
