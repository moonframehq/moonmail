use {
    moonmail_core::{BlobStore, MailboxRole},
    moonmail_network::{Account, IncomingMail, RemoteFolder, RemoteSnapshot, save_sent_copy},
    moonmail_store::{Archive, Draft, FileBlobs, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Mailbox {
    fail: bool,
    copies: Vec<(String, Vec<u8>)>,
}
impl IncomingMail for Mailbox {
    type Error = std::io::Error;
    fn folders(&mut self) -> std::io::Result<Vec<RemoteFolder>> {
        unreachable!()
    }
    fn snapshot(&mut self, _: &str) -> std::io::Result<RemoteSnapshot> {
        unreachable!()
    }
    fn fetch(&mut self, _: u32) -> std::io::Result<Option<Vec<u8>>> {
        unreachable!()
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> std::io::Result<()> {
        unreachable!()
    }
    fn append_sent(&mut self, folder: &str, raw: &[u8]) -> std::io::Result<bool> {
        self.copies.push((folder.to_owned(), raw.to_vec()));
        if self.fail {
            Err(std::io::Error::other("synthetic lost APPEND response"))
        } else {
            Ok(true)
        }
    }
}

fn accepted(archive: &mut Archive, blobs: &FileBlobs, draft_id: &str) -> Result<i64> {
    let draft = Draft {
        id: draft_id.into(),
        account: "me@example.org".into(),
        ..Default::default()
    };
    let id = archive.queue_with_message_id(&draft, &draft.account, &blobs.put(b"From: me@example.org\r\nTo: peer@example.org\r\nMessage-ID: <copy@example.org>\r\nSubject: Synthetic sent copy\r\n\r\nFrozen bytes")?, &["peer@example.org".into()], Some("copy@example.org"))?;
    archive.prepare_sent_copy(id, true)?;
    assert!(archive.claim_send(id)?);
    archive.send_result(id, "sent", "Accepted by SMTP")?;
    Ok(id)
}

#[test]
fn accepted_copy_is_exact_account_bound_and_never_repeated() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let id = accepted(&mut archive, &blobs, "first")?;
    let account = Account::fastmail("me@example.org")?;
    let mut remote = Mailbox {
        fail: false,
        copies: Vec::new(),
    };
    // Missing role preserves pending intent; sending to a guessed folder is forbidden.
    assert!(save_sent_copy(&mut remote, &account, &archive, &blobs, id).is_err());
    assert!(remote.copies.is_empty());
    let folder = archive.mailbox(&account.email, "Envoyés")?;
    archive.mailbox_server_role(folder, Some(MailboxRole::Sent))?;
    assert!(
        save_sent_copy(
            &mut remote,
            &Account::fastmail("other@example.org")?,
            &archive,
            &blobs,
            id
        )
        .is_err()
    );
    save_sent_copy(&mut remote, &account, &archive, &blobs, id)?;
    assert_eq!(
        remote.copies[0],
        ("Envoyés".into(), blobs.get(&archive.outbox_item(id)?.blob)?)
    );
    assert!(save_sent_copy(&mut remote, &account, &archive, &blobs, id).is_err());
    assert_eq!(remote.copies.len(), 1);
    assert_eq!(archive.outbox_item(id)?.state, "sent");
    assert_eq!(archive.outbox_item(id)?.sent_copy_state, "saved");
    assert!(archive.pending_sent_copies(&account.email)?.is_empty());
    Ok(())
}

#[test]
fn lost_append_and_restart_require_observation_never_smtp_or_append_retry() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.org")?;
    let folder = archive.mailbox(&account.email, "Sent")?;
    archive.mailbox_server_role(folder, Some(MailboxRole::Sent))?;
    let id = accepted(&mut archive, &blobs, "uncertain")?;
    let mut remote = Mailbox {
        fail: true,
        copies: Vec::new(),
    };
    assert!(save_sent_copy(&mut remote, &account, &archive, &blobs, id).is_err());
    assert_eq!(archive.outbox_item(id)?.sent_copy_state, "unknown");
    let interrupted = accepted(&mut archive, &blobs, "interrupted")?;
    assert!(archive.claim_sent_copy(interrupted, "Sent")?);
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    archive.recover_interrupted_sent_copies()?;
    assert_eq!(archive.outbox_item(interrupted)?.sent_copy_state, "unknown");
    assert!(archive.pending_sent_copies(&account.email)?.is_empty());
    assert!(archive.retry_outbox(id).is_err());
    assert!(save_sent_copy(&mut remote, &account, &archive, &blobs, id).is_err());
    assert_eq!(remote.copies.len(), 1);
    let raw = blobs.get(&archive.outbox_item(id)?.blob)?;
    archive.ingest(
        &moonmail_mime::StalwartParser::default(),
        &blobs,
        IncomingMessage {
            account: &account.email,
            mailbox: folder,
            origin: "observed-copy",
            epoch: 1,
            uid: Some(1),
            seen: true,
            starred: false,
            received_at: 1,
            raw: &raw,
        },
    )?;
    archive.reconcile_sent(&account.email)?;
    assert_eq!(archive.outbox_item(id)?.sent_copy_state, "saved");
    assert_eq!(archive.outbox_item(id)?.state, "sent");
    Ok(())
}
