#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("archive database operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("archive file operation failed")]
    Io(#[from] std::io::Error),
    #[error("archive data is invalid")]
    Json(#[from] serde_json::Error),
    #[error("archive schema version {0} is not supported")]
    Schema(i64),
    #[error("message not found")]
    NotFound,
    #[error("invalid archive operation: {0}")]
    Invalid(&'static str),
}

#[derive(Debug, thiserror::Error)]
pub enum IngestError<P: std::error::Error + 'static, B: std::error::Error + 'static> {
    #[error("could not extract MIME")]
    Parser(#[source] P),
    #[error("could not persist original message")]
    Blob(#[source] B),
    #[error(transparent)]
    Store(#[from] Error),
}

pub type Result<T> = std::result::Result<T, Error>;
