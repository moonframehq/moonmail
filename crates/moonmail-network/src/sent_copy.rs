use {
    crate::{Account, Error, IncomingMail, Result},
    moonmail_core::{BlobStore, MailRepository, MailboxRole},
    moonmail_store::{Archive, FileBlobs},
};

#[derive(Debug, thiserror::Error)]
pub enum SentCopyError<E: std::error::Error + 'static> {
    #[error(transparent)]
    Store(#[from] moonmail_store::Error),
    #[error(transparent)]
    Blob(#[from] moonmail_store::BlobError),
    #[error("Sent copy outcome is uncertain; Sync and inspect Sent before further action")]
    Incoming(#[source] E),
    #[error("IMAP adapter does not support saving a Sent copy")]
    Unsupported,
    #[error("This message is not eligible for a Sent copy")]
    Ineligible,
}

pub fn save_sent_copy<I: IncomingMail>(
    incoming: &mut I,
    account: &Account,
    archive: &Archive,
    blobs: &FileBlobs,
    id: i64,
) -> std::result::Result<(), SentCopyError<I::Error>> {
    let item = archive.outbox_item(id)?;
    if item.account != account.email || item.state != "sent" || item.sent_copy_state != "pending" {
        return Err(SentCopyError::Ineligible);
    }
    let folder = archive.role_mailbox(&account.email, MailboxRole::Sent)?;
    let mailbox = archive
        .mailboxes()?
        .into_iter()
        .find(|mailbox| mailbox.id == folder)
        .ok_or(moonmail_store::Error::Invalid(
            "Sent folder is unavailable; Sync or choose a Sent role first",
        ))?;
    let raw = blobs.get(&item.blob)?;
    if !archive.claim_sent_copy(id, &mailbox.name)? {
        return Err(SentCopyError::Ineligible);
    }
    match incoming.append_sent(&mailbox.name, &raw) {
        Ok(true) => archive.sent_copy_result(id, "saved", "Saved accepted message to IMAP Sent")?,
        Ok(false) => {
            archive.sent_copy_result(id, "pending", "IMAP adapter does not support APPEND")?;
            return Err(SentCopyError::Unsupported);
        },
        Err(error) => {
            archive.sent_copy_result(
                id,
                "unknown",
                "Connection failed during APPEND; Sync and inspect Sent. No automatic retry.",
            )?;
            return Err(SentCopyError::Incoming(error));
        },
    }
    Ok(())
}

pub(crate) fn save_native(
    incoming: &mut crate::RustImap,
    account: &Account,
    archive: &Archive,
    blobs: &FileBlobs,
    id: i64,
) -> Result<()> {
    save_sent_copy(incoming, account, archive, blobs, id).map_err(|error| match error {
        SentCopyError::Store(error) => error.into(),
        SentCopyError::Blob(error) => error.into(),
        SentCopyError::Incoming(error) => error,
        SentCopyError::Unsupported => Error::Invalid("IMAP adapter cannot save Sent copies"),
        SentCopyError::Ineligible => Error::Invalid("Sent copy is no longer pending"),
    })
}
