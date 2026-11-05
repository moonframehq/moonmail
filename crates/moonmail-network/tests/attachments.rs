use {
    moonmail_core::{BlobStore, MimeAttachments},
    moonmail_mime::MailparseParser,
    moonmail_network::{Account, prepare_message, prepare_message_with_attachments},
    moonmail_store::{Archive, Draft, DraftAttachment, FileBlobs},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn restart_safe_attachments_freeze_exact_bytes_in_mime_and_outbox() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::attachments(dir.path());
    let first = vec![0, 1, 2, 255, 254, 0];
    let second = b"Synthetic attachment\r\n".to_vec();
    let mut draft = Draft {
        id: "attached-draft".into(),
        account: "me@example.org".into(),
        recipients: "peer@example.org".into(),
        bcc: "hidden@example.org".into(),
        body: "Outer body".into(),
        ..Default::default()
    };
    for (number, bytes) in [&first, &second].into_iter().enumerate() {
        draft.attachments.push(DraftAttachment {
            id: number.to_string(),
            name: "café.bin".into(),
            blob: blobs.put(bytes)?,
            bytes: bytes.len() as u64,
        });
    }
    let archive = Archive::open(dir.path())?;
    archive.save_draft(&draft)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    let draft = archive.draft(&draft.id)?.ok_or("missing reopened draft")?;
    assert_eq!(draft.attachments.len(), 2);
    let account = Account::fastmail("me@example.org")?;
    assert!(
        prepare_message(&account, &draft).is_err(),
        "callers cannot accidentally omit files"
    );
    let (raw, envelope) = prepare_message_with_attachments(&account, &draft, &blobs)?;
    assert!(!String::from_utf8_lossy(&raw).contains("hidden@example.org"));
    let files = MailparseParser.attachments(&raw)?;
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].name, "café.bin");
    assert_eq!(files[1].name, files[0].name);
    assert_eq!(MailparseParser.attachment(&raw, &files[0].id)?, first);
    assert_eq!(MailparseParser.attachment(&raw, &files[1].id)?, second);
    let mime_blobs = FileBlobs::new(dir.path());
    let blob = mime_blobs.put(&raw)?;
    archive.queue(&draft, &account.email, &blob, &envelope)?;
    drop(archive);
    std::fs::remove_dir_all(dir.path().join("attachments"))?;
    let archive = Archive::open(dir.path())?;
    assert!(archive.draft(&draft.id)?.is_none());
    let item = archive.outbox()?.pop().ok_or("missing queued message")?;
    assert_eq!(
        mime_blobs.get(&item.blob)?,
        raw,
        "frozen MIME no longer depends on draft files"
    );
    assert_eq!(item.recipients, ["peer@example.org", "hidden@example.org"]);
    assert!(prepare_message_with_attachments(&account, &draft, &blobs).is_err());
    Ok(())
}

#[test]
fn corruption_size_mismatch_and_attachment_limits_fail_without_partial_mime() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::attachments(dir.path());
    let blob = blobs.put(b"file")?;
    let account = Account::fastmail("me@example.org")?;
    let mut draft = Draft {
        recipients: "peer@example.org".into(),
        attachments: vec![DraftAttachment {
            id: "one".into(),
            name: "file.bin".into(),
            blob: blob.clone(),
            bytes: 5,
        }],
        ..Default::default()
    };
    assert!(prepare_message_with_attachments(&account, &draft, &blobs).is_err());
    draft.attachments[0].bytes = 4;
    std::fs::write(
        dir.path()
            .join("attachments")
            .join(&blob[..2])
            .join(format!("{blob}.bin")),
        b"evil",
    )?;
    assert!(prepare_message_with_attachments(&account, &draft, &blobs).is_err());
    draft.attachments[0].bytes = moonmail_store::MAX_ATTACHMENT_BYTES + 1;
    assert!(draft.validate_attachments().is_err());
    draft.attachments[0].bytes = 4;
    draft.attachments = vec![draft.attachments[0].clone(); 17];
    assert!(draft.validate_attachments().is_err());
    Ok(())
}
