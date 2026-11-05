//! SQLite FTS5 search projection. Owns a dedicated, rebuildable database.

mod error;
mod parser;
mod sqlite;

pub use {
    error::{Error, Result},
    parser::{QueryError, parse_query},
    sqlite::SqliteIndex,
};
