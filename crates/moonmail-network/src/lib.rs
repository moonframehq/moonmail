//! Connection-owning mail workers and provider authentication.
mod account;
mod auth;
mod credentials;
mod error;
mod incoming;
mod submission;
mod sync;

pub use {
    account::{Account, validate_address},
    auth::{Auth, AuthError, LoginSecret, oauth_bearer},
    credentials::{CredentialStore, Keychain, KeychainError},
    error::{Error, Result},
    incoming::{IncomingMail, RemoteFolder, RemoteMessage, RemoteSnapshot, RustImap},
    submission::{SmtpSubmission, Submission, SubmitError, prepare_message, send_queued},
    sync::{SyncError, sync_account, synchronize},
};
