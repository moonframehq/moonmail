use {
    crate::{Account, Auth, Error, IncomingMail, Keychain, Result, RustImap},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
    std::{collections::HashSet, path::Path},
};

#[derive(Debug, thiserror::Error)]
pub enum SyncError<E: std::error::Error + 'static> {
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
    mut progress: impl FnMut(String),
) -> std::result::Result<(), SyncError<I::Error>> {
    let parser = StalwartParser::default();
    for folder in incoming.folders().map_err(SyncError::Incoming)? {
        progress(format!("Checking {}", folder.name));
        let mailbox = archive.mailbox(&account.email, &folder.name)?;
        archive.mailbox_hierarchy(mailbox, folder.delimiter.as_deref(), folder.selectable)?;
        if !folder.selectable {
            continue;
        }
        let snapshot = incoming
            .snapshot(&folder.name)
            .map_err(SyncError::Incoming)?;
        for operation in archive
            .pending_flags(&account.email)?
            .into_iter()
            .filter(|op| op.mailbox == folder.name)
        {
            if operation.epoch == snapshot.epoch {
                incoming
                    .flag(operation.uid, &operation.kind, operation.value)
                    .map_err(SyncError::Incoming)?;
            }
            // Reconcile using the local desired flags below; a subsequent sync
            // obtains the post-STORE server snapshot.
            archive.acknowledge_flag(&operation)?;
        }
        let snapshot = incoming
            .snapshot(&folder.name)
            .map_err(SyncError::Incoming)?;
        let flags: Vec<_> = snapshot
            .messages
            .iter()
            .map(|m| (m.uid, m.seen, m.starred))
            .collect();
        let known: HashSet<_> = archive
            .reconcile(mailbox, snapshot.epoch, &flags)?
            .into_iter()
            .collect();
        for message in snapshot
            .messages
            .iter()
            .rev()
            .filter(|message| !known.contains(&message.uid))
        {
            if message.size > 64 * 1024 * 1024 {
                progress(format!("Skipped UID {}: exceeds 64 MiB", message.uid));
                continue;
            }
            if let Some(raw) = incoming.fetch(message.uid).map_err(SyncError::Incoming)? {
                archive.ingest(
                    &parser,
                    blobs,
                    IncomingMessage {
                        account: &account.email,
                        mailbox,
                        origin: &format!("imap:{mailbox}:{}:{}", snapshot.epoch, message.uid),
                        epoch: snapshot.epoch,
                        uid: Some(message.uid),
                        seen: message.seen,
                        starred: message.starred,
                        received_at: time::OffsetDateTime::now_utc().unix_timestamp(),
                        raw: &raw,
                    },
                )?;
                progress(format!("Downloaded {} · UID {}", folder.name, message.uid));
            }
        }
    }
    Ok(())
}

pub fn sync_account(root: &Path, progress: impl FnMut(String)) -> Result<()> {
    let account = Account::load(root)?.ok_or(Error::Invalid("Connect an account first"))?;
    let secret = Auth::new(Keychain, root).credentials(&account.email)?;
    let mut incoming = RustImap::connect(&account, &secret)?;
    synchronize(
        &mut incoming,
        &account,
        &mut Archive::open(root)?,
        &FileBlobs::new(root),
        progress,
    )
    .map_err(|error| match error {
        SyncError::Incoming(error) => error,
        SyncError::Store(error) => error.into(),
        SyncError::Ingest(error) => error.into(),
    })
}
