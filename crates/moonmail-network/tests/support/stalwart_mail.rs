use {
    super::{Fixture, Peer, Result},
    moonmail_core::MailboxRole,
    moonmail_network::{IncomingMail, LiveAccount, LiveStatus, RustImap},
    moonmail_store::{Archive, SyncCheckpoint},
    std::time::{Duration, Instant},
};

fn checkpoint(peer: &Peer) -> Result<SyncCheckpoint> {
    let archive = Archive::open(peer.root.path())?;
    let folder = archive.role_mailbox(&peer.account.email, MailboxRole::Inbox)?;
    archive
        .sync_checkpoint(folder)?
        .ok_or_else(|| "Missing durable CONDSTORE checkpoint".into())
}

pub(super) fn watch(fixture: &Fixture, recipient: &Peer) -> Result<(LiveAccount, SyncCheckpoint)> {
    // An empty, never-modified mailbox can have HIGHESTMODSEQ zero. Start
    // from a nonempty UID range so the next delivery exercises a real delta.
    fixture.incoming(recipient)?.append_sent(
        "INBOX",
        b"From: seed@moonmail.test\r\nSubject: IDLE baseline\r\n\r\nSynthetic baseline\r\n",
    )?;
    let tls = fixture.imap.clone();
    let secret = recipient.secret.clone();
    let worker = LiveAccount::start_with(
        recipient.root.path().to_owned(),
        recipient.account.clone(),
        15,
        move |account| RustImap::connect_with_tls(account, &secret, &tls),
    )?;
    wait_for_idle(&worker)?;
    Ok((worker, checkpoint(recipient)?))
}

fn wait_for_idle(worker: &LiveAccount) -> Result {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        for event in worker.events() {
            match event {
                LiveStatus::Idle => return Ok(()),
                LiveStatus::Error { error, .. } => return Err(error.into()),
                _ => {},
            }
        }
        if Instant::now() >= deadline {
            return Err("Recipient never entered IMAP IDLE".into());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

pub(super) fn pushed(
    worker: LiveAccount,
    before: SyncCheckpoint,
    peer: &Peer,
    raw: &[u8],
) -> Result<moonmail_core::MessageId> {
    // The worker's polling interval is fifteen minutes. No manual sync is issued.
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        for event in worker.events() {
            match event {
                LiveStatus::Idle => {
                    if let Some(id) = peer.find_observed(raw, MailboxRole::Inbox, true)? {
                        let after = checkpoint(peer)?;
                        assert_eq!(before.epoch, after.epoch);
                        assert!(
                            after.upper > before.upper
                                && after.highest_modseq > before.highest_modseq
                        );
                        return Ok(id);
                    }
                },
                LiveStatus::Error { error, .. } => return Err(error.into()),
                _ => {},
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err("Recipient did not receive SMTP delivery through IMAP IDLE".into())
}

pub(super) fn verify_edits(
    fixture: &Fixture,
    sender: &Peer,
    recipient: &Peer,
    id: moonmail_core::MessageId,
) -> Result {
    use moonmail_store::{FolderEdit, MoveTarget};
    let mut db = Archive::open(recipient.root.path())?;
    let account = &recipient.account.email;
    let inbox = db.role_mailbox(account, MailboxRole::Inbox)?;
    let name = format!("Interop-{}", uuid::Uuid::new_v4());
    let renamed = format!("{name}-renamed");
    db.queue_folder_edit(account, FolderEdit::Create(name.clone()))?;
    db.set_flag(id, "seen", true)?;
    db.set_flag(id, "flagged", true)?;
    let tag = format!("project-{}", uuid::Uuid::new_v4().simple());
    db.set_keywords(&[id], &tag, true)?;
    fixture.sync(recipient)?;
    let folder = db.mailbox(account, &name)?;
    db.queue_copies(&[id], Some(inbox), MoveTarget::Folder(folder))?;
    fixture.sync(recipient)?;
    let mut witness = fixture.incoming(recipient)?;
    let copy = witness.snapshot(&name)?;
    assert_eq!(copy.messages.len(), 1);
    assert!(copy.messages[0].seen && copy.messages[0].starred);
    assert!(copy.messages[0].keywords.iter().any(|k| k == &tag));
    assert!(
        !fixture
            .incoming(sender)?
            .folders()?
            .iter()
            .any(|f| f.name == name)
    );
    db.queue_folder_edit(
        account,
        FolderEdit::Rename {
            id: folder,
            name: renamed.clone(),
        },
    )?;
    fixture.sync(recipient)?;
    assert_eq!(witness.snapshot(&renamed)?.messages.len(), 1);
    db.queue_moves(&[id], Some(inbox), MoveTarget::Role(MailboxRole::Trash))?;
    fixture.sync(recipient)?;
    let trash = witness
        .folders()?
        .into_iter()
        .find(|f| f.role == Some(MailboxRole::Trash))
        .ok_or("Missing trash folder")?;
    assert!(
        witness
            .snapshot(&trash.name)?
            .messages
            .iter()
            .any(|m| m.keywords.iter().any(|k| k == &tag))
    );
    assert!(
        !witness
            .snapshot("INBOX")?
            .messages
            .iter()
            .any(|m| m.keywords.iter().any(|k| k == &tag))
    );
    db.queue_folder_edit(account, FolderEdit::Delete(folder))?;
    fixture.sync(recipient)?;
    assert!(!witness.folders()?.iter().any(|f| f.name == renamed));
    Ok(())
}
