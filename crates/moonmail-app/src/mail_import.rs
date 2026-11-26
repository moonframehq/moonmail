//! Read local mail sources without changing the source. Each message is durable
//! before the next one is read, so a failed import can be retried.
use {
    anyhow::{Context, Result, ensure},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
    std::{
        fs::{self, File},
        io::{BufReader, Read},
        path::Path,
    },
};

mod mbox;
const LIMIT: usize = 64 * 1024 * 1024;

pub(super) fn from_path(path: &Path, root: &Path) -> Result<usize> {
    let source = path.canonicalize()?;
    let destination = root.canonicalize()?;
    ensure!(
        !destination.starts_with(&source) && !source.starts_with(&destination),
        "Choose a source outside the Moonmail archive"
    );
    run(path, &mut Archive::open(root)?, &FileBlobs::new(root))
}

fn run(path: &Path, archive: &mut Archive, blobs: &FileBlobs) -> Result<usize> {
    let mut reader = Import {
        archive,
        blobs,
        parser: StalwartParser::default(),
        count: 0,
    };
    reader
        .visit(path, "Imported", true)
        .with_context(|| format!("Importing {}", path.display()))?;
    Ok(reader.count)
}

struct Import<'a> {
    archive: &'a mut Archive,
    blobs: &'a FileBlobs,
    parser: StalwartParser,
    count: usize,
}

impl Import<'_> {
    fn visit(&mut self, path: &Path, folder: &str, explicit: bool) -> Result<()> {
        let kind = fs::symlink_metadata(path)?.file_type();
        if kind.is_dir() {
            let maildir = ["cur", "new"]
                .into_iter()
                .all(|dir| fs::symlink_metadata(path.join(dir)).is_ok_and(|m| m.is_dir()));
            if maildir {
                let mailbox = self.archive.mailbox("local", folder)?;
                for dir in ["cur", "new"] {
                    for entry in fs::read_dir(path.join(dir))? {
                        let entry = entry?;
                        if entry.file_type()?.is_file()
                            && !entry.file_name().as_encoded_bytes().starts_with(b".")
                        {
                            let name = entry.file_name();
                            let flags = name.to_string_lossy();
                            let flags = flags.rsplit_once(":2,").map_or("", |(_, flags)| flags);
                            self.single(
                                &entry.path(),
                                mailbox,
                                flags.contains('S'),
                                flags.contains('F'),
                            )?;
                        }
                    }
                }
            }
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if maildir {
                    if matches!(name.as_ref(), "cur" | "new" | "tmp")
                        || !entry.file_type()?.is_dir()
                    {
                        continue;
                    }
                    let child = name
                        .strip_prefix('.')
                        .map_or_else(|| name.to_string(), |n| n.replace('.', "/"));
                    self.visit(&entry.path(), &format!("{folder}/{child}"), false)?;
                } else {
                    let child = entry.path();
                    // Preserve legacy recursive .eml imports into Imported. A
                    // nested Maildir gets its own mailbox.
                    let child_folder = if entry.file_type()?.is_dir()
                        && child.join("cur").is_dir()
                        && child.join("new").is_dir()
                    {
                        format!("{folder}/{name}")
                    } else {
                        folder.to_owned()
                    };
                    self.visit(&child, &child_folder, false)?;
                }
            }
        } else if kind.is_file() {
            let extension = path.extension().unwrap_or_default();
            if extension.eq_ignore_ascii_case("eml") {
                let mailbox = self.archive.mailbox("local", folder)?;
                self.single(path, mailbox, true, false)?;
            } else if explicit
                || extension.eq_ignore_ascii_case("mbox")
                || extension.eq_ignore_ascii_case("mbx")
                || (path.file_name().is_some_and(|name| name == "mbox")
                    && path
                        .parent()
                        .and_then(Path::extension)
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("mbox")))
            {
                let file = File::open(path)?;
                let mailbox = self.archive.mailbox("local", folder)?;
                mbox::read(BufReader::new(file), |bytes| {
                    self.store(bytes, mailbox, true, false)
                })?;
            }
        }
        Ok(())
    }

    fn single(&mut self, path: &Path, mailbox: i64, seen: bool, starred: bool) -> Result<()> {
        let mut raw = Vec::new();
        File::open(path)?
            .take((LIMIT + 1) as u64)
            .read_to_end(&mut raw)?;
        ensure!(
            raw.len() <= LIMIT,
            "Message exceeds 64 MiB in {}",
            path.display()
        );
        self.store(&raw, mailbox, seen, starred)
    }

    fn store(&mut self, raw: &[u8], mailbox: i64, seen: bool, starred: bool) -> Result<()> {
        let origin = format!("import:{}", moonmail_store::content_digest(raw));
        self.archive.ingest(
            &self.parser,
            self.blobs,
            IncomingMessage {
                account: "local",
                mailbox,
                origin: &origin,
                epoch: 0,
                uid: None,
                seen,
                starred,
                received_at: 0,
                raw,
            },
        )?;
        self.count += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        moonmail_core::{MailQuery, MailRepository},
        tempfile::tempdir,
    };

    #[test]
    fn imports_maildir_folders_flags_and_eml_on_repeat() -> Result<()> {
        let source = tempdir()?;
        let root = tempdir()?;
        let mut archive = Archive::open(root.path())?;
        let blobs = FileBlobs::new(root.path());
        for path in ["cur", "new", "tmp", ".Lists.Rust/cur", ".Lists.Rust/new"] {
            fs::create_dir_all(source.path().join(path))?;
        }
        let bytes = b"Subject: Flags\r\n\r\nhello";
        fs::write(source.path().join("cur/unique:2,FS"), bytes)?;
        fs::write(
            source.path().join(".Lists.Rust/new/other"),
            b"Subject: New\n\nbody",
        )?;
        fs::write(source.path().join("tmp/in-flight.eml"), b"unfinished")?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(source.path(), source.path().join(".Loop"))?;
        for _ in 0..2 {
            assert_eq!(run(source.path(), &mut archive, &blobs)?, 2);
        }
        assert_eq!(archive.total()?, 2);
        let boxes = archive.mailboxes()?;
        assert!(
            boxes
                .iter()
                .any(|b| b.name == "Imported/Lists/Rust" && b.unread == 1)
        );
        let rows = archive.page(&MailQuery::default(), 20)?.rows;
        assert!(
            rows.iter()
                .any(|m| m.subject == "Flags" && m.starred && !m.unread)
        );
        let eml = source.path().join("copy.eml");
        fs::write(&eml, bytes)?;
        assert_eq!(run(&eml, &mut archive, &blobs)?, 1);
        assert_eq!(archive.total()?, 2);
        Ok(())
    }

    #[test]
    fn streams_mboxrd_and_recovers_original_message_bytes() -> Result<()> {
        let mut decoded = Vec::new();
        mbox::read(&b"From a Wed Jan  1 00:00:00 2025\r\nSubject: One\r\n\r\n>>From nested\r\n>From body\r\n\r\nFrom b Thu Jan  2 00:00:00 2025\nSubject: Two\n\nlast"[..], |msg| {
            decoded.push(msg.to_owned());
            Ok(())
        })?;
        assert_eq!(
            decoded,
            [
                b"Subject: One\r\n\r\n>From nested\r\nFrom body\r\n".to_vec(),
                b"Subject: Two\n\nlast".to_vec()
            ]
        );
        assert!(mbox::read(&b"invalid\n"[..], |_| Ok(())).is_err());
        Ok(())
    }
}
