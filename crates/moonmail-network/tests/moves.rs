use {
    moonmail_core::{BlobStore, MailQuery, MailRepository, MailboxRole},
    moonmail_network::{
        Account, CopyOutcome, IncomingMail, MoveOutcome, RemoteFolder, RemoteMessage,
        RemoteSnapshot, synchronize,
    },
    moonmail_store::{Archive, FileBlobs, MoveTarget},
    std::{collections::BTreeMap, io},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Server {
    folders: BTreeMap<String, Vec<u32>>,
    selected: String,
    epoch: u32,
    moves: usize,
    flags: usize,
    lost_ack: bool,
    fail_select: bool,
    supported: bool,
    cancel_on_move: Option<std::rc::Rc<std::cell::Cell<bool>>>,
    fallback: &'static str,
    copies: usize,
    removals: usize,
}

impl Server {
    fn new() -> Self {
        Self {
            folders: BTreeMap::from([
                ("INBOX".into(), vec![1]),
                ("Conserves".into(), vec![]),
                ("Corbeille".into(), vec![]),
            ]),
            selected: String::new(),
            epoch: 7,
            moves: 0,
            flags: 0,
            lost_ack: false,
            fail_select: false,
            supported: true,
            cancel_on_move: None,
            fallback: "unsupported",
            copies: 0,
            removals: 0,
        }
    }
}

impl IncomingMail for Server {
    type Error = io::Error;
    fn folders(&mut self) -> io::Result<Vec<RemoteFolder>> {
        Ok(self
            .folders
            .keys()
            .map(|name| RemoteFolder {
                name: name.clone(),
                delimiter: Some("/".into()),
                selectable: true,
                role: match name.as_str() {
                    "INBOX" => Some(MailboxRole::Inbox),
                    "Conserves" => Some(MailboxRole::Archive),
                    "Corbeille" => Some(MailboxRole::Trash),
                    _ => None,
                },
            })
            .collect())
    }
    fn snapshot(&mut self, name: &str) -> io::Result<RemoteSnapshot> {
        if self.fail_select {
            return Err(io::Error::other("connection lost before dispatch"));
        }
        self.selected = name.into();
        let messages = self
            .folders
            .get(name)
            .ok_or_else(|| io::Error::other("missing folder"))?
            .iter()
            .map(|uid| RemoteMessage {
                keywords: Vec::new(),
                received_at: 1_700_000_000 + i64::from(*uid),
                uid: *uid,
                seen: false,
                starred: false,
                size: 200,
            })
            .collect();
        Ok(RemoteSnapshot {
            epoch: self.epoch
                + u32::from(self.fallback == "source_epoch" && self.copies > 0 && name == "INBOX"),
            messages,
        })
    }
    fn fetch(&mut self, _: u32) -> io::Result<Option<Vec<u8>>> {
        if self.selected == "Conserves" && self.fallback == "different_bytes" {
            return Ok(Some(b"From: sender@example.org\r\nSubject: Different destination\r\n\r\nDifferent synthetic bytes".to_vec()));
        }
        Ok(Some(b"From: sender@example.org\r\nTo: me@example.org\r\nSubject: Synthetic move\r\nMessage-ID: <move@example.org>\r\n\r\nImmutable synthetic body".to_vec()))
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> io::Result<()> {
        self.flags += 1;
        Ok(())
    }
    fn move_message(&mut self, uid: u32, destination: &str) -> io::Result<MoveOutcome> {
        if !self.supported {
            return Ok(MoveOutcome::Unsupported);
        }
        self.moves += 1;
        self.folders
            .get_mut(&self.selected)
            .ok_or_else(|| io::Error::other("no selection"))?
            .retain(|id| *id != uid);
        self.folders
            .get_mut(destination)
            .ok_or_else(|| io::Error::other("missing destination"))?
            .push(uid + 100);
        if let Some(stop) = &self.cancel_on_move {
            stop.set(true);
        }
        if self.lost_ack {
            return Err(io::Error::other("remote move committed, response lost"));
        }
        Ok(MoveOutcome::Moved)
    }

    fn copy_for_move(&mut self, uid: u32, destination: &str) -> io::Result<CopyOutcome> {
        if self.fallback == "unsupported" {
            return Ok(CopyOutcome::Unsupported);
        }
        self.copies += 1;
        self.folders
            .get_mut(destination)
            .ok_or_else(|| io::Error::other("missing destination"))?
            .push(uid + 100);
        if self.fallback == "lost_copy" {
            return Err(io::Error::other("copy committed, response lost"));
        }
        if self.fallback == "no_receipt" {
            return Ok(CopyOutcome::Unverified);
        }
        if self.fallback == "cancel_copy"
            && let Some(stop) = &self.cancel_on_move
        {
            stop.set(true);
        }
        Ok(CopyOutcome::Copied {
            epoch: self.epoch + u32::from(self.fallback == "destination_epoch"),
            uid: uid + 100,
        })
    }

    fn copy_message(&mut self, uid: u32, destination: &str) -> io::Result<CopyOutcome> {
        self.copy_for_move(uid, destination)
    }

    fn remove_copied_source(&mut self, uid: u32) -> io::Result<MoveOutcome> {
        if self.fallback == "no_remove" {
            return Ok(MoveOutcome::Unsupported);
        }
        assert_eq!(self.selected, "INBOX");
        self.removals += 1;
        self.folders
            .get_mut(&self.selected)
            .ok_or_else(|| io::Error::other("missing source"))?
            .retain(|id| *id != uid);
        if self.fallback == "lost_remove" {
            return Err(io::Error::other("source removal committed, response lost"));
        }
        if self.fallback == "cancel_remove"
            && let Some(stop) = &self.cancel_on_move
        {
            stop.set(true);
        }
        Ok(MoveOutcome::Moved)
    }
}

#[test]
fn standalone_copy_retains_source_records_outcomes_before_cancellation_and_never_replays() -> Result
{
    for scenario in [
        "success",
        "no_receipt",
        "lost_copy",
        "cancel_copy",
        "cancel_before",
        "unsupported",
        "epoch",
        "crash",
        "record_failure",
    ] {
        let dir = tempfile::tempdir()?;
        let blobs = FileBlobs::new(dir.path());
        let account = Account::fastmail("me@example.org")?;
        let mut archive = Archive::open(dir.path())?;
        let mut server = Server::new();
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        let source = archive.mailbox(&account.email, "INBOX")?;
        let id = archive.page(&MailQuery::default(), 10)?.rows[0].id;
        archive.queue_copies(&[id], Some(source), MoveTarget::Role(MailboxRole::Archive))?;
        let operation = archive.move_operations()?[0].id;
        if scenario == "record_failure" {
            let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
            db.execute_batch("CREATE TRIGGER fail_copy_completion BEFORE UPDATE OF state ON move_operations WHEN NEW.state='done' BEGIN SELECT RAISE(ABORT,'synthetic completion failure'); END;")?;
        }
        if scenario == "crash" {
            assert!(archive.claim_move(operation)?);
        }
        drop(archive);
        let mut archive = Archive::open(dir.path())?;
        archive.recover_interrupted_moves()?;
        assert_eq!(archive.move_operations()?[0].kind, "copy");
        server.fallback = scenario;
        if scenario == "epoch" {
            server.epoch += 1;
        }
        let stop = std::rc::Rc::new(std::cell::Cell::new(false));
        server.cancel_on_move = Some(stop.clone());
        let result = moonmail_network::synchronize_cancellable(
            &mut server,
            &account,
            &mut archive,
            &blobs,
            |status| {
                if scenario == "cancel_before" && status.starts_with("Copying") {
                    stop.set(true);
                }
            },
            || stop.get(),
        );
        assert_eq!(
            result.is_err(),
            matches!(
                scenario,
                "lost_copy" | "cancel_copy" | "cancel_before" | "record_failure"
            ),
            "{scenario}"
        );
        let expected = match scenario {
            "lost_copy" | "crash" => "unknown",
            "cancel_before" => "queued",
            "unsupported" | "epoch" => "failed",
            "record_failure" => "running",
            _ => "done",
        };
        assert_eq!(archive.move_operations()?[0].state, expected, "{scenario}");
        assert_eq!(server.folders["INBOX"], vec![1]);
        assert_eq!(server.moves + server.removals, 0);
        assert_eq!(
            archive
                .page(
                    &MailQuery {
                        mailbox: Some(source),
                        ..Default::default()
                    },
                    10
                )?
                .rows
                .len(),
            1,
            "source retained: {scenario}"
        );
        if scenario == "cancel_before" {
            archive.cancel_move(operation)?;
        }
        let copies = server.copies;
        drop(archive);
        let mut archive = Archive::open(dir.path())?;
        archive.recover_interrupted_moves()?;
        server.fallback = "success";
        server.cancel_on_move = None;
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        assert_eq!(server.copies, copies, "no duplicate: {scenario}");
        let destination = archive.role_mailbox(&account.email, MailboxRole::Archive)?;
        assert_eq!(
            archive
                .page(
                    &MailQuery {
                        mailbox: Some(destination),
                        ..Default::default()
                    },
                    10
                )?
                .rows
                .len(),
            copies,
            "destination discovered: {scenario}"
        );
    }
    Ok(())
}

#[test]
fn copy_fallback_verifies_bytes_and_epochs_before_targeted_removal_and_never_retries_uncertainty()
-> Result {
    for scenario in [
        "success",
        "lost_copy",
        "no_receipt",
        "destination_epoch",
        "different_bytes",
        "source_epoch",
        "no_remove",
        "lost_remove",
        "cancel_copy",
        "cancel_remove",
        "record_failure",
    ] {
        let dir = tempfile::tempdir()?;
        let blobs = FileBlobs::new(dir.path());
        let account = Account::fastmail("me@example.org")?;
        let mut archive = Archive::open(dir.path())?;
        let mut server = Server::new();
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        let id = archive.page(&MailQuery::default(), 10)?.rows[0].id;
        archive.queue_moves(&[id], None, MoveTarget::Role(MailboxRole::Archive))?;
        if scenario == "record_failure" {
            let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
            db.execute_batch("CREATE TRIGGER fail_copy_receipt BEFORE UPDATE OF detail ON move_operations WHEN NEW.detail LIKE 'COPY acknowledged%' BEGIN SELECT RAISE(ABORT,'synthetic receipt persistence failure'); END;")?;
        }
        server.supported = false;
        server.fallback = scenario;
        let stop = std::rc::Rc::new(std::cell::Cell::new(false));
        server.cancel_on_move = Some(stop.clone());
        let result = moonmail_network::synchronize_cancellable(
            &mut server,
            &account,
            &mut archive,
            &blobs,
            |_| {},
            || stop.get(),
        );
        assert_eq!(
            result.is_err(),
            matches!(
                scenario,
                "lost_copy" | "lost_remove" | "cancel_copy" | "cancel_remove" | "record_failure"
            ),
            "{scenario}"
        );
        assert_eq!(server.copies, 1, "{scenario}");
        let removed = matches!(scenario, "success" | "lost_remove" | "cancel_remove");
        assert_eq!(server.removals, usize::from(removed), "{scenario}");
        assert_eq!(server.folders["INBOX"].is_empty(), removed, "{scenario}");
        let operation = archive.move_operations()?[0].clone();
        assert_eq!(
            operation.state,
            if matches!(scenario, "success" | "cancel_remove") {
                "done"
            } else if scenario == "record_failure" {
                "running"
            } else {
                "unknown"
            },
            "{scenario}"
        );
        if !matches!(scenario, "no_receipt" | "lost_copy" | "record_failure") {
            assert!(
                operation
                    .detail
                    .contains("COPY acknowledged destination UIDVALIDITY")
            );
        }
        drop(archive);
        archive = Archive::open(dir.path())?;
        archive.recover_interrupted_moves()?;
        stop.set(false);
        server.cancel_on_move = None;
        server.fallback = "success";
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        assert_eq!(server.copies, 1, "{scenario}: no second copy");
        assert_eq!(
            server.removals,
            usize::from(removed),
            "{scenario}: no second removal"
        );
    }
    Ok(())
}

#[test]
fn offline_move_survives_reopen_applies_flags_and_refreshes_destination_in_same_sync() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.org")?;
    let mut archive = Archive::open(dir.path())?;
    let mut server = Server::new();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    let id = archive.page(&MailQuery::default(), 10)?.rows[0].id;
    let original = archive.detail(id)?.blob;
    archive.set_flag(id, "seen", true)?;
    archive.queue_moves(&[id], None, MoveTarget::Role(MailboxRole::Archive))?;
    assert!(
        archive.set_flag(id, "flagged", true).is_err(),
        "later flags cannot race a frozen move"
    );
    assert_eq!(
        archive.page(&MailQuery::default(), 10)?.rows.len(),
        1,
        "queued source stays visible"
    );
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    archive.recover_interrupted_moves()?;
    server.fail_select = true;
    assert!(synchronize(&mut server, &account, &mut archive, &blobs, |_| {}).is_err());
    assert_eq!(archive.move_operations()?[0].state, "queued");
    server.fail_select = false;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.moves, 1);
    assert_eq!(server.flags, 1);
    assert_eq!(archive.move_operations()?[0].state, "done");
    assert!(archive.pending_flags(&account.email)?.is_empty());
    let destination = archive.role_mailbox(&account.email, MailboxRole::Archive)?;
    assert_eq!(
        archive
            .page(
                &MailQuery {
                    mailbox: Some(destination),
                    ..Default::default()
                },
                10
            )?
            .rows
            .len(),
        1
    );
    assert_eq!(archive.page(&MailQuery::default(), 10)?.rows.len(), 1);
    assert_eq!(archive.detail(id)?.blob, original);
    assert!(blobs.get(&original)?.ends_with(b"Immutable synthetic body"));
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.moves, 1, "completed move never repeats");
    Ok(())
}

#[test]
fn cancellation_after_remote_move_records_outcome_before_stopping_and_never_repeats_it() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.org")?;
    let mut archive = Archive::open(dir.path())?;
    let mut server = Server::new();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    let id = archive.page(&MailQuery::default(), 10)?.rows[0].id;
    archive.queue_moves(&[id], None, MoveTarget::Role(MailboxRole::Archive))?;
    let stop = std::rc::Rc::new(std::cell::Cell::new(false));
    assert!(matches!(
        moonmail_network::synchronize_cancellable(
            &mut server,
            &account,
            &mut archive,
            &blobs,
            |status| {
                if status.starts_with("Moving") {
                    stop.set(true);
                }
            },
            || stop.get()
        ),
        Err(moonmail_network::SyncError::Cancelled)
    ));
    assert_eq!(archive.move_operations()?[0].state, "queued");
    assert_eq!(server.moves, 0);
    stop.set(false);
    server.cancel_on_move = Some(stop.clone());
    assert!(matches!(
        moonmail_network::synchronize_cancellable(
            &mut server,
            &account,
            &mut archive,
            &blobs,
            |_| {},
            || stop.get()
        ),
        Err(moonmail_network::SyncError::Cancelled)
    ));
    assert_eq!(archive.move_operations()?[0].state, "done");
    assert_eq!(server.moves, 1);
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    archive.recover_interrupted_moves()?;
    server.cancel_on_move = None;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.moves, 1);
    assert_eq!(archive.page(&MailQuery::default(), 10)?.rows.len(), 1);
    Ok(())
}

#[test]
fn lost_move_response_is_uncertain_and_never_retried_after_restart() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.org")?;
    let mut archive = Archive::open(dir.path())?;
    let mut server = Server::new();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    let id = archive.page(&MailQuery::default(), 10)?.rows[0].id;
    archive.queue_moves(&[id], None, MoveTarget::Role(MailboxRole::Trash))?;
    server.lost_ack = true;
    assert!(synchronize(&mut server, &account, &mut archive, &blobs, |_| {}).is_err());
    let operation = archive.move_operations()?[0].clone();
    assert_eq!(operation.state, "unknown");
    assert!(!archive.claim_move(operation.id)?);
    assert!(archive.cancel_move(operation.id).is_err());
    assert!(
        archive
            .queue_moves(&[id], None, MoveTarget::Role(MailboxRole::Archive))
            .is_err()
    );
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    archive.recover_interrupted_moves()?;
    server.lost_ack = false;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.moves, 1);
    assert_eq!(archive.move_operations()?[0].state, "unknown");
    assert_eq!(
        archive.page(&MailQuery::default(), 10)?.rows.len(),
        1,
        "destination recovers via ordinary sync"
    );
    archive.dismiss_unknown_move(operation.id)?;
    assert_eq!(archive.move_operations()?[0].state, "dismissed");
    Ok(())
}

#[test]
fn changed_epoch_missing_destination_unsupported_and_cancelled_moves_never_mutate_remote_mail()
-> Result {
    for scenario in ["epoch", "destination", "unsupported", "cancelled", "crash"] {
        let dir = tempfile::tempdir()?;
        let blobs = FileBlobs::new(dir.path());
        let account = Account::fastmail("me@example.org")?;
        let mut archive = Archive::open(dir.path())?;
        let mut server = Server::new();
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        let id = archive.page(&MailQuery::default(), 10)?.rows[0].id;
        archive.queue_moves(&[id], None, MoveTarget::Role(MailboxRole::Archive))?;
        let operation = archive.move_operations()?[0].id;
        match scenario {
            "epoch" => server.epoch += 1,
            "destination" => {
                server.folders.remove("Conserves");
            },
            "unsupported" => server.supported = false,
            "cancelled" => archive.cancel_move(operation)?,
            "crash" => {
                assert!(archive.claim_move(operation)?);
                drop(archive);
                archive = Archive::open(dir.path())?;
                archive.recover_interrupted_moves()?;
            },
            _ => unreachable!(),
        }
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        assert_eq!(server.moves, 0, "{scenario}");
        let expected = match scenario {
            "cancelled" => "cancelled",
            "crash" => "unknown",
            _ => "failed",
        };
        assert_eq!(archive.move_operations()?[0].state, expected, "{scenario}");
        assert_eq!(server.folders["INBOX"], vec![1]);
    }
    Ok(())
}
