use {
    crate::{
        Account, Auth, Error, Keychain, Result, SmtpSubmission, Submission, SubmissionFailure,
    },
    moonmail_core::BlobStore,
    moonmail_store::{Archive, FileBlobs},
    std::path::Path,
};

#[derive(Debug, thiserror::Error)]
pub enum DeliveryError<S: std::error::Error + 'static, B: std::error::Error + 'static> {
    #[error(transparent)]
    Store(#[from] moonmail_store::Error),
    #[error("frozen message could not be read")]
    Blob(#[source] B),
    #[error("submission failed; inspect Outbox status")]
    Submission(#[source] S),
    #[error("outbox account or state does not allow submission")]
    Ineligible,
}

pub type DeliveryResult<S, B> = std::result::Result<(), DeliveryError<S, B>>;

/// One explicit attempt using exact frozen bytes. Claim competes atomically with
/// cancellation/restoration; failure to claim must never contact the adapter.
pub fn transmit_queued<S: Submission, B: BlobStore>(
    archive: &Archive,
    account: &str,
    id: i64,
    blobs: &B,
    submission: &mut S,
) -> DeliveryResult<S::Error, B::Error> {
    let item = archive.outbox_item(id)?;
    if item.account != account || item.state != "queued" {
        return Err(DeliveryError::Ineligible);
    }
    let raw = blobs.get(&item.blob).map_err(DeliveryError::Blob)?;
    if !archive.claim_send(id)? {
        return Err(DeliveryError::Ineligible);
    }
    match submission.submit(&item.sender, &item.recipients, &raw) {
        Ok(()) => archive.send_result(id, "sent", "Accepted by SMTP server")?,
        Err(error) => {
            let state = match S::failure(&error) {
                SubmissionFailure::NotSubmitted => "failed",
                SubmissionFailure::Unknown => "unknown",
            };
            archive.send_result(id, state, &error.to_string())?;
            return Err(DeliveryError::Submission(error));
        },
    }
    Ok(())
}

pub fn send_queued(root: &Path, id: i64) -> Result<()> {
    let archive = Archive::open(root)?;
    let item = archive.outbox_item(id)?;
    let account = Account::load_for(root, &item.account)?
        .ok_or(Error::Invalid("Reconnect the message's sending account"))?;
    if item.account != account.email || item.state != "queued" {
        return Err(Error::Invalid(
            "Outbox account or state does not allow submission",
        ));
    }
    // Validate the item before touching credentials or refreshing a token.
    let secret = Auth::new(Keychain).credentials_for(&account)?;
    archive.prepare_sent_copy(id, account.sent_copy == crate::SentCopy::Append)?;
    let email = account.email.clone();
    transmit_queued(
        &archive,
        &email,
        id,
        &FileBlobs::new(root),
        &mut SmtpSubmission {
            account: account.clone(),
            secret: secret.clone(),
        },
    )?;
    if account.sent_copy == crate::SentCopy::Append {
        let result = (|| {
            let mut incoming = crate::RustImap::connect(&account, &secret)?;
            crate::sent_copy::save_native(
                &mut incoming,
                &account,
                &archive,
                &FileBlobs::new(root),
                id,
            )
        })();
        result.map_err(|error| Error::SentCopyPending(Box::new(error)))?;
    }
    Ok(())
}
