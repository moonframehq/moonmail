use moonmail_core::MessageId;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("SQLite search operation failed")]
    Sqlite(#[from] rusqlite::Error),
    #[error("message ID {0:?} exceeds SQLite's signed integer range")]
    MessageIdOutOfRange(MessageId),
    #[error("search limit exceeds SQLite's signed integer range")]
    LimitOutOfRange,
    #[error("search database schema version {0} is not supported")]
    SchemaVersion(i64),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
