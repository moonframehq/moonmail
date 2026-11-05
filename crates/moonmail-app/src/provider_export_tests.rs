use super::{tests::data, *};

#[test]
fn selection_exports_are_scoped_complete_hash_verified_and_importable() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let source = archive.mailbox("local", "Imported")?;
    let blobs = FileBlobs::new(dir.path());
    let mut ids = Vec::new();
    let mut originals = Vec::new();
    for n in 0..2 {
        let raw=format!("From: peer@example.org\r\nSubject: Export {n}\r\n\r\nFrom original line\r\n>From quoted line\r\n").into_bytes();
        ids.push(archive.ingest(
            &moonmail_mime::StalwartParser::default(),
            &blobs,
            moonmail_store::IncomingMessage {
                account: "local",
                mailbox: source,
                origin: &n.to_string(),
                epoch: 0,
                uid: None,
                seen: false,
                starred: true,
                received_at: n,
                raw: &raw,
            },
        )?);
        originals.push(raw);
    }
    drop(archive);
    crate::fonts::Preferences {
        mark_read: false,
        ..Default::default()
    }
    .save(dir.path())?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    mail.query.mailbox = Some(source);
    mail.checked.extend(ids.iter().copied());
    let request =
        json!({"action":"messages_save","messages":ids,"scope":source,"paths":[output.path()]});
    mail.checked.clear();
    assert!(mail.handle(data(request.clone())).is_err());
    mail.checked.extend(ids.iter().copied());
    mail.query.mailbox = None;
    assert!(mail.handle(data(request.clone())).is_err());
    assert_eq!(std::fs::read_dir(output.path())?.count(), 0);
    mail.query.mailbox = Some(source);
    mail.handle(data(request.clone()))?;
    let exported = std::fs::read_dir(output.path())?
        .next()
        .context("missing export")??;
    let manifest: serde_json::Value =
        serde_json::from_slice(&std::fs::read(exported.path().join("manifest.json"))?)?;
    assert_eq!(manifest["format"], "moonmail-eml-export");
    assert_eq!(
        manifest["messages"]
            .as_array()
            .context("manifest entries")?
            .len(),
        2
    );
    for (n, id) in ids.iter().enumerate() {
        assert_eq!(
            std::fs::read(exported.path().join(format!("message-{}.eml", id.0)))?,
            originals[n]
        );
        assert_eq!(
            manifest["messages"][n]["sha256"],
            mail.archive()?.message_blob(*id)?
        );
        assert!(
            mail.archive()?.detail(*id)?.summary.unread,
            "export does not mark read"
        );
    }
    mail.handle(data(request.clone()))?;
    assert_eq!(
        std::fs::read_dir(output.path())?.count(),
        2,
        "repeated export creates a separate directory"
    );
    let restored = tempfile::tempdir()?;
    crate::cli::dispatch(
        restored.path(),
        &[
            "import".into(),
            exported.path().to_string_lossy().into_owned(),
        ],
    )?;
    let restored_archive = Archive::open(restored.path())?;
    assert_eq!(restored_archive.total()?, 2);
    for row in restored_archive.page(&MailQuery::default(), 10)?.rows {
        let raw = FileBlobs::new(restored.path()).get(&restored_archive.message_blob(row.id)?)?;
        assert!(originals.contains(&raw));
    }
    let hash = mail.archive()?.message_blob(ids[1])?;
    std::fs::write(
        dir.path()
            .join("messages")
            .join(&hash[..2])
            .join(format!("{hash}.eml")),
        b"corrupt",
    )?;
    assert!(mail.handle(data(request)).is_err());
    assert_eq!(
        std::fs::read_dir(output.path())?.count(),
        2,
        "failed second message removes only the incomplete new export"
    );
    assert!(exported.path().join("manifest.json").is_file());
    Ok(())
}

#[test]
fn eml_export_preserves_original_bytes_rejects_stale_panels_and_never_overwrites() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    let raw=b"From: Peer <peer@example.org>\r\nTo: you@example.com\r\nSubject: =?UTF-8?Q?Original_=C3=A9?=\r\nX-Original: folded\r\n\tvalue\r\nContent-Type: multipart/mixed; boundary=fixture\r\n\r\n--fixture\r\nContent-Type: text/plain\r\n\r\nFrom a synthetic body\r\n--fixture\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\nAP/+AQ==\r\n--fixture--\r\n";
    let mut ids = Vec::new();
    for n in 0..2 {
        ids.push(archive.ingest(
            &moonmail_mime::StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            moonmail_store::IncomingMessage {
                account: "you@example.com",
                mailbox: inbox,
                origin: &n.to_string(),
                epoch: 0,
                uid: None,
                seen: true,
                starred: false,
                received_at: n,
                raw,
            },
        )?);
    }
    drop(archive);
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    mail.preferences.mark_read = false;
    mail.select(ids[0], true)?;
    let output = dir.path().join("original.eml");
    let request = json!({"action":"message_save","message":ids[0],"path":output});
    mail.select(ids[1], true)?;
    assert!(mail.handle(data(request.clone())).is_err());
    assert!(!output.exists());
    mail.select(ids[0], true)?;
    mail.handle(data(request.clone()))?;
    assert_eq!(std::fs::read(&output)?, raw);
    assert!(mail.handle(data(request)).is_err());
    assert_eq!(std::fs::read(&output)?, raw);
    assert!(!mail.archive()?.detail(ids[0])?.summary.unread);
    let cli_output = dir.path().join("cli.eml");
    crate::cli::dispatch(
        dir.path(),
        &[
            "export".into(),
            ids[0].0.to_string(),
            cli_output.to_string_lossy().into_owned(),
        ],
    )?;
    assert_eq!(std::fs::read(&cli_output)?, raw);
    #[cfg(unix)]
    {
        let alias = dir.path().join("alias.eml");
        std::os::unix::fs::symlink(&output, &alias)?;
        assert!(crate::file_export::save_new(&alias, b"replacement").is_err());
        assert_eq!(std::fs::read(&output)?, raw);
    }
    let restored = tempfile::tempdir()?;
    crate::cli::dispatch(
        restored.path(),
        &["import".into(), output.to_string_lossy().into_owned()],
    )?;
    let restored_archive = Archive::open(restored.path())?;
    let restored_id = restored_archive.page(&MailQuery::default(), 1)?.rows[0].id;
    assert_eq!(
        FileBlobs::new(restored.path()).get(&restored_archive.message_blob(restored_id)?)?,
        raw
    );
    let hash = mail.archive()?.message_blob(ids[0])?;
    std::fs::write(
        dir.path()
            .join("messages")
            .join(&hash[..2])
            .join(format!("{hash}.eml")),
        b"corrupted fixture",
    )?;
    let corrupt_output = dir.path().join("corrupt.eml");
    assert!(
        mail.handle(data(
            json!({"action":"message_save","message":ids[0],"path":corrupt_output})
        ))
        .is_err()
    );
    assert!(!corrupt_output.exists());
    Ok(())
}
