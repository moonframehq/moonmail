#[derive(Debug, thiserror::Error)]
pub enum Error {
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
    Ingest(
        #[from]
        moonmail_store::IngestError<moonmail_mime::StalwartError, moonmail_store::BlobError>,
    ),
    #[error(transparent)]
    Auth(#[from] crate::AuthError<crate::KeychainError>),
    #[error("{0}")]
    Invalid(&'static str),
}
pub type Result<T> = std::result::Result<T, Error>;
