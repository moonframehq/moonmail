//! SQLite FTS5 search projection. Owns a dedicated, rebuildable database.

mod error;
mod sqlite;

pub use {
    error::{Error, Result},
    sqlite::SqliteIndex,
};
