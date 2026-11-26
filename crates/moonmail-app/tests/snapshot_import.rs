//! Synthetic Unix mailbox fixtures exercised through the same CLI as `just import`.
use {
    anyhow::{Result, ensure},
    moonmail_core::{BlobStore, MailRepository},
    moonmail_store::{Archive, FileBlobs, content_digest},
    std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
        process::{Command, Output},
    },
    tempfile::tempdir,
};

fn import(source: &Path, archive: &Path) -> Result<Output> {
    Ok(Command::new(env!("CARGO_BIN_EXE_moonmail"))
        .env("MOONMAIL_DATA_DIR", archive)
        .args(["import".as_ref(), source.as_os_str()])
        .output()?)
}

fn contents(dir: &Path, files: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            files.insert(path.clone(), None);
            contents(&path, files)?;
        } else {
            files.insert(path.clone(), Some(fs::read(path)?));
        }
    }
    Ok(())
}

#[test]
fn imports_mixed_unix_snapshot_read_only() -> Result<()> {
    let temp = tempdir()?;
    let source = temp.path().join("mail");
    let root = temp.path().join("archive");
    let mail = b"Subject: binary\r\nContent-Transfer-Encoding: 8bit\r\n\r\n\xff\0\r\n";
    for folder in [
        "Home/Projects/2025",
        "Work/Projects/2025",
        "Personal",
        "Personal/..Lists.Rust",
    ] {
        let dir = source.join(folder);
        for subdir in ["cur", "new", "tmp"] {
            fs::create_dir_all(dir.join(subdir))?;
        }
        fs::write(dir.join("cur/1735689600.M42.host,U=17:2,FRS"), mail)?;
        fs::write(dir.join(".uidvalidity"), b"1735689600\n18\n")?;
        fs::write(dir.join(".mbsyncstate"), b"FarUidValidity 1735689600\n")?;
        fs::write(dir.join("tmp/delivery.eml"), b"in flight")?;
    }
    let eml = b"Subject: loose\r\n\tfolded\r\n\r\nno final newline\x80";
    fs::write(source.join("Home/Projects/export.EML"), eml)?;

    let first = b"Subject: mixed\n\n>From quote\xff\n";
    let empty = b"Subject: empty\r\n\r\n";
    let mut spool = b"From alice@example.test Wed Jan  1 00:00:00 2025\nSubject: mixed\n\n>>From quote\xff\n\r\nFrom bob@example.test Thu Jan  2 00:00:00 2025\n".to_vec();
    spool.extend_from_slice(empty);
    fs::write(source.join("saved.mbox"), spool)?;

    let body = b"From the train\n>From a quote\n\xff\x00\n\n";
    let mut record = format!("Subject: length\ncontent-length: {}\n\n", body.len()).into_bytes();
    record.extend_from_slice(body);
    let length_box = source.join("unix-spool");
    let mut framed = b"From alice@example.test Wed Jan  1 00:00:00 2025\n".to_vec();
    framed.extend_from_slice(&record);
    framed.extend_from_slice(b"\nFrom bob@example.test Thu Jan  2 00:00:00 2025\n");
    framed.extend_from_slice(empty);
    fs::write(&length_box, framed)?;

    let mut before = BTreeMap::new();
    contents(&source, &mut before)?;
    for _ in 0..2 {
        for path in [&source, &length_box] {
            let result = import(path, &root)?;
            ensure!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
    }
    let mut after = BTreeMap::new();
    contents(&source, &mut after)?;
    assert_eq!(before, after);

    let archive = Archive::open(&root)?;
    assert_eq!(archive.total()?, 5);
    let blobs = FileBlobs::new(&root);
    for bytes in [mail.as_slice(), eml, first, empty, &record] {
        assert_eq!(blobs.get(&content_digest(bytes))?, bytes);
    }
    let boxes = archive.mailboxes()?;
    for name in [
        "Imported/Home/Projects/2025",
        "Imported/Work/Projects/2025",
        "Imported/Personal",
        "Imported/Personal/INBOX/Lists/Rust",
    ] {
        assert!(
            boxes
                .iter()
                .any(|b| b.name == name && b.total == 1 && b.unread == 0),
            "{name}"
        );
    }
    assert!(boxes.iter().any(|b| b.name == "Imported" && b.total == 4));
    Ok(())
}

#[test]
fn rejects_overlapping_archive_before_creating_any_files() -> Result<()> {
    let source = tempdir()?;
    let root = source.path().join("archive/nested");
    assert!(!import(source.path(), &root)?.status.success());
    assert_eq!(fs::read_dir(source.path())?.count(), 0);
    Ok(())
}
