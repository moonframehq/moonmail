use {
    crate::{
        Account, Auth, Error, IncomingMail, InventoryBatch, Keychain, Result, RustImap, ScanError,
        ScanOutcome,
    },
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
    mut progress: impl FnMut(String),
    cancelled: impl Fn() -> bool,
) -> std::result::Result<(), SyncError<I::Error>> {
    check_cancel(&cancelled)?;
    let parser = StalwartParser::default();
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
    let mut skipped = 0usize;
    for folder in folders {
        progress(format!("Checking {}", folder.name));
        check_cancel(&cancelled)?;
        let mailbox = archive.mailbox(&account.email, &folder.name)?;
        if !folder.selectable {
            continue;
        }
        let (epoch, modseq) = inventory(incoming, archive, mailbox, &folder.name, &cancelled)?;
        let mut changed_flags = false;
        for operation in archive
            .pending_flags(&account.email)?
            .into_iter()
            .filter(|op| op.mailbox == folder.name)
        {
            check_cancel(&cancelled)?;
            if operation.epoch == epoch {
                incoming
                    .flag(operation.uid, &operation.kind, operation.value)
                    .map_err(SyncError::Incoming)?;
                changed_flags = true;
            }
            // Reconcile using the local desired flags below; a subsequent sync
            // obtains the post-STORE server snapshot.
            archive.acknowledge_flag(&operation)?;
        }
        check_cancel(&cancelled)?;
        let (epoch, modseq) = if changed_flags {
            inventory(incoming, archive, mailbox, &folder.name, &cancelled)?
        } else {
            (epoch, modseq)
        };
        let mut before = u64::from(u32::MAX) + 1;
        loop {
            check_cancel(&cancelled)?;
            let messages = archive.missing_sync_messages(before)?;
            if messages.is_empty() {
                break;
            }
            let mut remaining = messages.as_slice();
            while let Some(first) = remaining.first() {
                check_cancel(&cancelled)?;
                if first.size > BODY_LIMIT {
                    before = u64::from(first.uid);
                    skipped += 1;
                    progress(format!("Skipped UID {}: exceeds 64 MiB", first.uid));
                    remaining = &remaining[1..];
                    continue;
                }
                let count = body_batch_len(remaining);
                let (batch, rest) = remaining.split_at(count);
                remaining = rest;
                let uids: Vec<_> = batch.iter().map(|m| m.uid).collect();
                let mut bodies = if count > 1 {
                    incoming.fetch_batch(&uids).map_err(SyncError::Incoming)?
                } else {
                    Default::default()
                };
                for message in batch {
                    check_cancel(&cancelled)?;
                    before = u64::from(message.uid);
                    let raw = match bodies.remove(&message.uid) {
                        Some(raw) => raw,
                        None => match incoming.fetch(message.uid).map_err(SyncError::Incoming)? {
                            Some(raw) => raw,
                            None => continue,
                        },
                    };
                    check_cancel(&cancelled)?;
                    archive.ingest_staged(
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
                    progress(format!("Downloaded {} · UID {}", folder.name, message.uid));
                }
            }
        }
        check_cancel(&cancelled)?;
        if let Some(modseq) = modseq {
            archive.publish_sync_checkpoint(mailbox, modseq)?;
        }
    }
    check_cancel(&cancelled)?;
    archive.reconcile_sent(&account.email)?;
    if skipped > 0 {
        Err(SyncError::Incomplete(skipped))
    } else {
        Ok(())
    }
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

const BODY_LIMIT: u32 = 64 * 1024 * 1024;
// The first eligible message may exceed the ordinary 8 MiB batch budget.
fn body_batch_len(messages: &[SyncMessage]) -> usize {
    let mut bytes = 0u64;
    messages
        .iter()
        .take(32)
        .take_while(|m| {
            bytes += u64::from(m.size);
            bytes <= 8 * 1024 * 1024
        })
        .count()
        .max(1)
}

fn inventory<I: IncomingMail>(
    incoming: &mut I,
    archive: &mut Archive,
    mailbox: i64,
    folder: &str,
    cancelled: &impl Fn() -> bool,
) -> std::result::Result<(u32, Option<u64>), SyncError<I::Error>> {
    let previous = archive.begin_mailbox_inventory(mailbox)?;
    let outcome = incoming
        .scan_changes(folder, previous, |epoch, upper, batch| {
            check_cancel(cancelled)?;
            match batch {
                InventoryBatch::Metadata(rows) => {
                    archive.stage_sync_inventory(epoch, upper, rows)?
                },
                InventoryBatch::Live(uids) => archive.stage_sync_live(epoch, upper, uids)?,
                InventoryBatch::Flags(flags) => archive.stage_sync_flags(epoch, upper, flags)?,
                InventoryBatch::Vanished(lo, hi) => {
                    archive.stage_sync_vanished(epoch, upper, lo, hi)?
                },
            }
            check_cancel(cancelled)
        })
        .map_err(|error| match error {
            ScanError::Incoming(error) => SyncError::Incoming(error),
            ScanError::Sink(error) => error,
        })?;
    check_cancel(cancelled)?;
    match outcome {
        ScanOutcome::Unchanged(base) => Ok((base.epoch, None)),
        ScanOutcome::Complete { kind, modseq } => {
            Ok((archive.merge_sync_inventory(mailbox, kind)?, modseq))
        },
    }
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
    // One connection-owning worker visits accounts in registration order. Stop
    // on the first failure; completed account work is already durable.
    for account in accounts {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        progress(format!("Syncing {}", account.email));
        sync_connected_account(root, &account, &mut progress, &cancelled)?;
    }
    Ok(())
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
    synchronize_cancellable(
        &mut incoming,
        account,
        &mut archive,
        &FileBlobs::new(root),
        progress,
        &cancelled,
    )
    .map_err(|error| match error {
        SyncError::Incoming(error) => error,
        SyncError::Store(error) => error.into(),
        SyncError::Ingest(error) => error.into(),
        SyncError::Cancelled => Error::Cancelled,
        SyncError::Incomplete(count) => Error::IncompleteSync(count),
    })?;
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
            crate::sent_copy::save_native(
                &mut incoming,
                account,
                &archive,
                &FileBlobs::new(root),
                id,
            )
            .map_err(|error| Error::SentCopyPending(Box::new(error)))?;
        }
    }
    Ok(())
}
