use {
    moonmail_core::{MailboxRole, MessageId},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, Draft, FileBlobs, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn ingest(
    archive: &mut Archive,
    blobs: &FileBlobs,
    account: &str,
    mailbox: i64,
    uid: Option<u32>,
) -> Result<MessageId> {
    Ok(archive.ingest(&StalwartParser::default(), blobs, IncomingMessage {
        account, mailbox, origin: &format!("{mailbox}:{uid:?}"), epoch: 7, uid, seen: true, starred: false, received_at: 0,
        raw: b"From: me@example.org\r\nTo: peer@example.org\r\nMessage-ID: <frozen@moonmail.local>\r\nSubject: Synthetic sent tracking\r\n\r\nOriginal body",
    })?)
}

#[test]
fn sent_observation_is_account_role_and_remote_occurrence_scoped_without_changing_delivery()
-> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let draft = Draft {
        id: "uncertain".into(),
        account: "me@example.org".into(),
        ..Default::default()
    };
    let id = archive.queue_with_message_id(
        &draft,
        &draft.account,
        "frozen",
        &["peer@example.org".into()],
        Some("frozen@moonmail.local"),
    )?;
    assert!(archive.claim_send(id)?);
    archive.send_result(id, "unknown", "Acknowledgment lost")?;
    archive.reconcile_sent(&draft.account)?;
    assert_eq!(archive.outbox_item(id)?.sent_status, "unavailable");
    let sent = archive.mailbox(&draft.account, "Enviados")?;
    archive.mailbox_server_role(sent, Some(MailboxRole::Sent))?;
    let inbox = archive.mailbox(&draft.account, "INBOX")?;
    let other = archive.mailbox("other@example.org", "Sent")?;
    archive.mailbox_server_role(other, Some(MailboxRole::Sent))?;
    ingest(&mut archive, &blobs, &draft.account, inbox, Some(1))?;
    ingest(&mut archive, &blobs, "other@example.org", other, Some(1))?;
    ingest(&mut archive, &blobs, &draft.account, sent, None)?;
    archive.reconcile_sent(&draft.account)?;
    assert_eq!(archive.outbox_item(id)?.sent_status, "missing");
    let message = ingest(&mut archive, &blobs, &draft.account, sent, Some(2))?;
    archive.reconcile_sent(&draft.account)?;
    let item = archive.outbox_item(id)?;
    assert_eq!(item.sent_status, "visible");
    assert_eq!(item.sent_message, Some(message));
    assert_eq!(item.state, "unknown");
    assert!(!item.editable);
    assert!(archive.retry_outbox(id).is_err());
    assert!(archive.restore_outbox_draft(id, "replacement").is_err());
    assert!(!archive.claim_send(id)?);
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert_eq!(archive.outbox_item(id)?.sent_message, Some(message));
    ingest(&mut archive, &blobs, &draft.account, sent, Some(3))?;
    archive.reconcile_sent(&draft.account)?;
    assert_eq!(archive.outbox_item(id)?.sent_status, "ambiguous");
    assert!(archive.outbox_item(id)?.sent_message.is_none());
    archive.reconcile(sent, 8, &[])?;
    archive.reconcile_sent(&draft.account)?;
    assert_eq!(archive.outbox_item(id)?.sent_status, "missing");
    assert_eq!(archive.outbox_item(id)?.state, "unknown");
    Ok(())
}

#[test]
fn duplicate_roles_are_not_guessed_and_explicit_override_selects_only_one_folder() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let draft = Draft {
        id: "accepted".into(),
        account: "me@example.org".into(),
        ..Default::default()
    };
    let id = archive.queue_with_message_id(
        &draft,
        &draft.account,
        "frozen",
        &[],
        Some("frozen@moonmail.local"),
    )?;
    assert!(archive.claim_send(id)?);
    archive.send_result(id, "sent", "SMTP accepted")?;
    let sent = archive.mailbox(&draft.account, "Sent")?;
    let second = archive.mailbox(&draft.account, "Sent2")?;
    for mailbox in [sent, second] {
        archive.mailbox_server_role(mailbox, Some(MailboxRole::Sent))?;
    }
    ingest(&mut archive, &blobs, &draft.account, second, Some(1))?;
    archive.reconcile_sent(&draft.account)?;
    assert_eq!(archive.outbox_item(id)?.sent_status, "unavailable");
    archive.set_mailbox_role(sent, Some(MailboxRole::Sent))?;
    archive.reconcile_sent(&draft.account)?;
    assert_eq!(archive.outbox_item(id)?.sent_status, "missing");
    archive.set_mailbox_role(second, Some(MailboxRole::Sent))?;
    archive.reconcile_sent(&draft.account)?;
    assert_eq!(archive.outbox_item(id)?.sent_status, "visible");
    assert_eq!(archive.outbox_item(id)?.state, "sent");
    Ok(())
}
