use {
    super::{tests::data, *},
    moonmail_core::{MimeAttachments, MimeParser},
};

#[test]
fn forwarding_preserves_files_but_starts_a_new_conversation_without_hidden_recipients() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    let raw = format!(
        "Bcc: hidden@example.org\nReferences: <root@example.org>\n{}",
        include_str!("../../moonmail-mime/tests/fixtures/attachments.eml")
    );
    let id = archive.ingest(
        &moonmail_mime::StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        moonmail_store::IncomingMessage {
            account: "you@example.com",
            mailbox: inbox,
            origin: "forward",
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 1,
            raw: raw.as_bytes(),
        },
    )?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    mail.selected = Some(mail.archive()?.detail(id)?);
    mail.handle(data(json!({"action":"compose"})))?;
    let previous = mail.draft.as_ref().context("missing previous")?.id.clone();
    mail.handle(data(json!({"action":"forward"})))?;
    let draft = mail.draft.as_ref().context("missing forward")?;
    assert_ne!(draft.id, previous);
    assert!(draft.recipients.is_empty() && draft.cc.is_empty() && draft.bcc.is_empty());
    assert!(draft.reply_id.is_empty() && draft.references.is_empty());
    assert_eq!(draft.attachments.len(), 4);
    assert_eq!(draft.subject, "Fwd: Synthetic attachment review");
    assert!(!draft.body.contains("hidden@example.org"));
    assert!(draft.body.contains("From: \"Clara\" <clara@example.org>"));
    let mut draft = draft.clone();
    draft.recipients = "new@example.org".into();
    let (forward, envelope) = moonmail_network::prepare_message_with_attachments(
        &Account::fastmail("you@example.com")?,
        &draft,
        &FileBlobs::attachments(dir.path()),
    )?;
    assert_eq!(envelope, ["new@example.org"]);
    let parsed = moonmail_mime::StalwartParser::default().parse(&forward)?;
    assert!(parsed.references.is_empty() && parsed.in_reply_to.is_none());
    let parser = moonmail_mime::MailparseParser;
    let original = parser.attachments(raw.as_bytes())?;
    let forwarded = parser.attachments(&forward)?;
    for (original, forwarded) in original.iter().zip(&forwarded) {
        assert_eq!(
            parser.attachment(raw.as_bytes(), &original.id)?,
            parser.attachment(&forward, &forwarded.id)?
        );
    }
    assert!(mail.archive()?.draft(&previous)?.is_some());
    mail.handle(data(json!({"action":"forward_text"})))?;
    assert!(
        mail.draft
            .as_ref()
            .context("missing text forward")?
            .attachments
            .is_empty()
    );
    assert!(mail.archive()?.draft(&draft.id)?.is_some());
    Ok(())
}

#[test]
fn oversized_forward_is_explicit_and_text_only_does_not_copy_files() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    let raw = format!("From: sender@example.org\r\nSubject: Fwd: Existing\r\nContent-Type: multipart/mixed; boundary=x\r\n\r\n--x\r\nContent-Type: text/plain\r\n\r\n{}\r\n{}--x--\r\n", "é".repeat(70_000), "--x\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=file.bin\r\n\r\nx\r\n".repeat(17));
    let id = archive.ingest(
        &moonmail_mime::StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        moonmail_store::IncomingMessage {
            account: "you@example.com",
            mailbox: inbox,
            origin: "oversized",
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 1,
            raw: raw.as_bytes(),
        },
    )?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    mail.selected = Some(mail.archive()?.detail(id)?);
    assert!(mail.handle(data(json!({"action":"forward"}))).is_err());
    assert_eq!(mail.archive()?.draft_count()?, 0);
    mail.handle(data(json!({"action":"forward_text"})))?;
    let draft = mail.draft.as_ref().context("missing forward")?;
    assert_eq!(draft.subject, "Fwd: Existing");
    assert!(draft.body.contains("[Forwarded text truncated]"));
    assert!(draft.body.chars().count() < 65_000);
    assert!(draft.attachments.is_empty());
    Ok(())
}
