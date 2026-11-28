//! The agent interface: one table of tools, served as `moonmail agent` (JSON
//! on stdout) and `moonmail mcp` (Model Context Protocol over stdio).
//!
//! Every tool opens the archive as an ordinary second SQLite writer, so it
//! works whether or not the app is open. Writes go through the same store
//! methods the UI uses and are logged in the archive's activity table; the
//! running app notices the commit (`PRAGMA data_version`), refreshes and
//! uploads queued remote changes. Without the app they upload on `moonmail sync`.

mod args;
pub mod cli;
pub mod mcp;
mod read;
mod requests;
pub(crate) mod tools;
pub mod workspace;
mod write;
