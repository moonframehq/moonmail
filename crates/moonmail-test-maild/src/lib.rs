//! Ephemeral IMAP/SMTP fixture daemon. No DNS delivery or external services.
mod imap;
mod server;
mod smtp;
mod store;
mod wire;

pub use {
    server::{Config, Endpoints, Server},
    store::{ACCOUNTS, PASSWORD},
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("test daemon I/O failed")]
    Io(#[from] std::io::Error),
    #[error("test daemon TLS failed")]
    Tls(#[from] rustls::Error),
    #[error("test certificate generation failed")]
    Certificate(#[from] rcgen::Error),
    #[error("test daemon configuration serialization failed")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(&'static str),
}

pub type Result<T = ()> = std::result::Result<T, Error>;
