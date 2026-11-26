use {
    crate::{Account, Auth, Error, IncomingMail, Keychain, Result, RustImap, ScanError},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage, SyncMessage},
    std::path::Path,
};

#[derive(Debug, thiserror::Error)]
pub enum SyncError<E: std::error::Error + 'static> {
    #[error("Sync cancelled at a command boundary; completed durable work is retained")]
    Cancelled,
    #[error("Sync is incomplete: {0} messages exceed the 64 MiB download limit")]
    Incomplete(usize),
    #[error("incoming mail operation failed")]
    Incoming(#[source] E),
    #[error(transparent)]
    Store(#[from] moonmail_store::Error),
    #[error(transparent)]
    Ingest(
        #[from]
        moonmail_store::IngestError<moonmail_mime::StalwartError, moonmail_store::BlobError>,
    ),
}

pub fn synchronize<I: IncomingMail>(
    incoming: &mut I,
    account: &Account,
    archive: &mut Archive,
    blobs: &FileBlobs,
    progress: impl FnMut(String),
) -> std::result::Result<(), SyncError<I::Error>> {
    synchronize_cancellable(incoming, account, archive, blobs, progress, || false)
}

pub(crate) fn check_cancel<E: std::error::Error + 'static>(
    cancelled: &impl Fn() -> bool,
) -> std::result::Result<(), SyncError<E>> {
    if cancelled() {
        Err(SyncError::Cancelled)
    } else {
        Ok(())
    }
}

/// Cancellation is checked between blocking commands, never between a remote
/// mutation and recording its outcome. Successful durable work remains resumable.
/// An in-flight command (or credential acquisition) must return before stopping.
pub fn synchronize_cancellable<I: IncomingMail>(
    incoming: &mut I,
    account: &Account,
    archive: &mut Archive,
    blobs: &FileBlobs,
    progress: impl FnMut(String),
    cancelled: impl Fn() -> bool,
) -> std::result::Result<(), SyncError<I::Error>> {
    synchronize_prioritized(
        incoming,
        account,
        archive,
        blobs,
        progress,
        cancelled,
        || false,
    )
    .map(|_| ())
}

fn synchronize_prioritized<I: IncomingMail>(
    incoming: &mut I,
    account: &Account,
    archive: &mut Archive,
    blobs: &FileBlobs,
    mut progress: impl FnMut(String),
    cancelled: impl Fn() -> bool,
    writes_waiting: impl Fn() -> bool,
) -> std::result::Result<Option<String>, SyncError<I::Error>> {
    check_cancel(&cancelled)?;
    let mut folders = incoming.folders().map_err(SyncError::Incoming)?;
    check_cancel(&cancelled)?;
    publish_folders(archive, &account.email, &folders)?;
    if crate::folder_operations::apply(
        incoming,
        &account.email,
        &folders,
        archive,
        &mut progress,
        &cancelled,
    )? {
        check_cancel(&cancelled)?;
        folders = incoming.folders().map_err(SyncError::Incoming)?;
        check_cancel(&cancelled)?;
        publish_folders(archive, &account.email, &folders)?;
    }
    crate::keywords::apply(incoming, &account.email, archive, &mut progress, &cancelled)?;
    crate::moves::apply_moves(incoming, &account.email, archive, &mut progress, &cancelled)?;
    // Flush durable writes before downloading any bodies. Probe the exact UID
    // and its epoch on the selected mailbox before issuing STORE.
    for operation in archive.pending_flags(&account.email)? {
        check_cancel(&cancelled)?;
        let source = incoming
            .select_uid(&operation.mailbox, operation.uid)
            .map_err(SyncError::Incoming)?;
        if source.epoch == operation.epoch && source.exists {
            incoming
                .flag(operation.uid, &operation.kind, operation.value)
                .map_err(SyncError::Incoming)?;
        }
        archive.acknowledge_flag(&operation)?;
    }
    // Yield inventory to writes only after finishing in-flight transfers.
    let cancelled = || cancelled() || writes_waiting();
    let mut skipped = 0usize;
    // Leave INBOX selected for the live worker. Re-selecting after the scan
    // would open a gap in which an arrival could be missed.
    folders.sort_by_key(|folder| folder.name.eq_ignore_ascii_case("INBOX"));
    let inbox_selected = folders
        .last()
        .filter(|f| f.selectable && f.name.eq_ignore_ascii_case("INBOX"))
        .map(|f| f.name.clone());
    for folder in folders {
        if folder.selectable {
            skipped += sync_folder(
                incoming,
                account,
                archive,
                blobs,
                &folder.name,
                &mut progress,
                cancelled,
            )?;
        }
    }
    check_cancel(&cancelled)?;
    archive.reconcile_sent(&account.email)?;
    if skipped > 0 {
        Err(SyncError::Incomplete(skipped))
    } else {
        Ok(inbox_selected)
    }
}

/// Refresh one selected mailbox after a push using its durable UID inventory.
pub(crate) fn sync_folder<I: IncomingMail>(
    incoming: &mut I,
    account: &Account,
    archive: &mut Archive,
    blobs: &FileBlobs,
    folder: &str,
    mut progress: impl FnMut(String),
    cancelled: impl Fn() -> bool,
) -> std::result::Result<usize, SyncError<I::Error>> {
    let mut skipped = 0;
    let parser = StalwartParser::default();
    progress(format!("Checking {folder}"));
    check_cancel(&cancelled)?;
    let mailbox = archive.mailbox(&account.email, folder)?;
    let epoch = inventory(incoming, archive, mailbox, folder, &cancelled)?;
    let mut before = u64::from(u32::MAX) + 1;
    loop {
        check_cancel(&cancelled)?;
        let messages = archive.missing_sync_messages(before)?;
        if messages.is_empty() {
            break;
        }
        for message in messages {
            before = u64::from(message.uid);
            check_cancel(&cancelled)?;
            if message.size > 64 * 1024 * 1024 {
                skipped += 1;
                progress(format!("Skipped UID {}: exceeds 64 MiB", message.uid));
                continue;
            }
            if let Some(raw) = incoming.fetch(message.uid).map_err(SyncError::Incoming)? {
                check_cancel(&cancelled)?;
                archive.ingest_with_keywords(
                    &parser,
                    blobs,
                    IncomingMessage {
                        account: &account.email,
                        mailbox,
                        origin: &format!("imap:{mailbox}:{epoch}:{}", message.uid),
                        epoch,
                        uid: Some(message.uid),
                        seen: message.seen,
                        starred: message.starred,
                        received_at: message.received_at,
                        raw: &raw,
                    },
                    &message.keywords,
                )?;
                progress(format!("Downloaded {} · UID {}", folder, message.uid));
            }
        }
    }
    Ok(skipped)
}

fn publish_folders(
    archive: &mut Archive,
    account: &str,
    folders: &[crate::RemoteFolder],
) -> moonmail_store::Result<()> {
    archive.reconcile_mailboxes(account, folders.iter().map(|folder| folder.name.as_str()))?;
    for folder in folders {
        let mailbox = archive.mailbox(account, &folder.name)?;
        archive.mailbox_hierarchy(mailbox, folder.delimiter.as_deref(), folder.selectable)?;
        archive.mailbox_server_role(mailbox, folder.role)?;
    }
    Ok(())
}

fn inventory<I: IncomingMail>(
    incoming: &mut I,
    archive: &mut Archive,
    mailbox: i64,
    folder: &str,
    cancelled: &impl Fn() -> bool,
) -> std::result::Result<u32, SyncError<I::Error>> {
    archive.begin_sync_inventory()?;
    incoming
        .scan(folder, |epoch, upper, batch| {
            check_cancel(cancelled)?;
            if batch.len() > 1000 {
                return Err(SyncError::Store(moonmail_store::Error::Invalid(
                    "Incoming adapter exceeded inventory batch limit",
                )));
            }
            let messages: Vec<_> = batch
                .iter()
                .map(|m| SyncMessage {
                    keywords: m.keywords.clone(),
                    uid: m.uid,
                    seen: m.seen,
                    starred: m.starred,
                    size: m.size,
                    received_at: m.received_at,
                })
                .collect();
            archive.stage_sync_inventory(epoch, upper, &messages)?;
            check_cancel(cancelled)
        })
        .map_err(|error| match error {
            ScanError::Incoming(error) => SyncError::Incoming(error),
            ScanError::Sink(error) => error,
        })?;
    check_cancel(cancelled)?;
    Ok(archive.finish_sync_inventory(mailbox)?)
}

pub fn sync_account(root: &Path, progress: impl FnMut(String)) -> Result<()> {
    sync_account_cancellable(root, progress, || false)
}

pub fn sync_account_cancellable(
    root: &Path,
    mut progress: impl FnMut(String),
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let accounts = Account::load_all(root)?;
    if accounts.is_empty() {
        return Err(Error::Invalid("Connect an account first"));
    }
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        thread,
        time::Duration,
    };
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        let (sender, events) = mpsc::sync_channel(64);
        for account in accounts {
            let sender = sender.clone();
            let stop = &stop;
            thread::Builder::new()
                .name("moonmail-sync".into())
                .spawn_scoped(scope, move || {
                    let report = |message| {
                        let _ = sender.send(Ok(format!("{} · {message}", account.email)));
                    };
                    let result = sync_connected_account(root, &account, report, || {
                        stop.load(Ordering::Acquire)
                    });
                    if let Err(source) = result {
                        let _ = sender.send(Err(Error::AccountSync {
                            account: account.email,
                            source: Box::new(source),
                        }));
                    }
                })?;
        }
        drop(sender);
        let mut failure = None;
        loop {
            if cancelled() {
                stop.store(true, Ordering::Release);
            }
            match events.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(message)) => progress(message),
                Ok(Err(error)) => {
                    progress(error.to_string());
                    failure.get_or_insert(error);
                },
                Err(mpsc::RecvTimeoutError::Timeout) => {},
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        failure.map_or(Ok(()), Err)
    })
}

fn sync_connected_account(
    root: &Path,
    account: &Account,
    progress: impl FnMut(String),
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    let secret = Auth::new(Keychain).credentials_for(account)?;
    if cancelled() {
        return Err(Error::Cancelled);
    }
    let mut incoming = RustImap::connect(account, &secret)?;
    let mut archive = Archive::open(root)?;
    sync_session(
        root,
        account,
        &mut incoming,
        &mut archive,
        progress,
        cancelled,
        || false,
    )
    .map(|_| ())
}

pub(crate) fn sync_session(
    root: &Path,
    account: &Account,
    incoming: &mut RustImap,
    archive: &mut Archive,
    progress: impl FnMut(String),
    cancelled: impl Fn() -> bool,
    writes_waiting: impl Fn() -> bool,
) -> Result<Option<String>> {
    let inbox_selected = synchronize_prioritized(
        incoming,
        account,
        archive,
        &FileBlobs::new(root),
        progress,
        &cancelled,
        writes_waiting,
    )?;
    loop {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let pending = archive.pending_sent_copies(&account.email)?;
        if pending.is_empty() {
            break;
        }
        for id in pending {
            if cancelled() {
                return Err(Error::Cancelled);
            }
            crate::sent_copy::save_native(incoming, account, archive, &FileBlobs::new(root), id)
                .map_err(|error| Error::SentCopyPending(Box::new(error)))?;
        }
    }
    Ok(inbox_selected)
}

impl From<SyncError<Error>> for Error {
    fn from(error: SyncError<Error>) -> Self {
        match error {
            SyncError::Incoming(error) => error,
            SyncError::Store(error) => error.into(),
            SyncError::Ingest(error) => error.into(),
            SyncError::Cancelled => Error::Cancelled,
            SyncError::Incomplete(count) => Error::IncompleteSync(count),
        }
    }
}
