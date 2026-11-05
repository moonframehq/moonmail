#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Sync cancelled; durable completed work is retained")]
    Cancelled,
    #[error("Sync is incomplete: {0} messages exceed the 64 MiB download limit")]
    IncompleteSync(usize),
    #[error("mail connection failed")]
    Imap(#[from] imap::Error),
    #[error("TLS connection failed")]
    Tls(#[from] native_tls::Error),
    #[error("mail file or socket operation failed")]
    Io(#[from] std::io::Error),
    #[error("invalid account data")]
    Json(#[from] serde_json::Error),
    #[error("invalid recipient address")]
    Address(#[from] mailparse::MailParseError),
    #[error(transparent)]
    Store(#[from] moonmail_store::Error),
    #[error(transparent)]
    Blob(#[from] moonmail_store::BlobError),
    #[error(transparent)]
    Delivery(#[from] crate::DeliveryError<crate::SubmitError, moonmail_store::BlobError>),
    #[error(transparent)]
    Ingest(
        #[from]
        moonmail_store::IngestError<moonmail_mime::StalwartError, moonmail_store::BlobError>,
    ),
    #[error(transparent)]
    Auth(#[from] crate::AuthError<crate::KeychainError>),
    #[error(transparent)]
    Submission(#[from] crate::SubmitError),
    #[error("SMTP accepted the message, but saving its Sent copy failed; inspect Outbox and Sync")]
    SentCopyPending(#[source] Box<Error>),
    #[error("{0}")]
    Invalid(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// Only transport failures are eligible for background sync reconnect.
    /// Authentication, TLS validation, local storage and incomplete sync require
    /// explicit intervention. Submission errors never qualify.
    pub fn retryable_sync(&self) -> bool {
        match self {
            Self::Imap(imap::Error::ConnectionLost | imap::Error::Io(_)) => true,
            Self::Io(error) => matches!(
                error.kind(),
                std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::NetworkUnreachable
                    | std::io::ErrorKind::HostUnreachable
            ),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_classification_excludes_local_and_nontransport_failures() {
        assert!(Error::Imap(imap::Error::ConnectionLost).retryable_sync());
        assert!(Error::Io(std::io::ErrorKind::ConnectionRefused.into()).retryable_sync());
        assert!(!Error::Io(std::io::ErrorKind::PermissionDenied.into()).retryable_sync());
        assert!(!Error::IncompleteSync(1).retryable_sync());
        assert!(!Error::Cancelled.retryable_sync());
        assert!(!Error::Invalid("Reconnect required").retryable_sync());
        assert!(!Error::Store(moonmail_store::Error::Invalid("disk failure")).retryable_sync());
    }
}
