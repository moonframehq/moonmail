//! Connection-owning mail workers and provider authentication.
mod account;
mod account_connection;
mod auth;
mod composition;
mod credentials;
mod delivery;
mod error;
mod folder_operations;
mod folders;
mod forwarding;
mod identities;
mod imap_copy;
mod imap_folders;
mod imap_inventory;
mod imap_keywords;
mod imap_tls;
mod incoming;
mod keywords;
mod moves;
mod recipients;
mod sent_copy;
mod submission;
mod sync;

pub use {
    account::{Account, Provider, SentCopy, TlsMode, validate_address},
    account_connection::check_account,
    auth::{Auth, AuthError, LoginSecret, oauth_bearer},
    composition::{ReplyMode, compose_reply, compose_reply_with_identities},
    credentials::{CredentialStore, Keychain, KeychainError},
    delivery::{DeliveryError, DeliveryResult, send_queued, transmit_queued},
    error::{Error, Result},
    forwarding::compose_forward,
    identities::{choose_identity, insert_signature, validate_identity},
    incoming::{
        CopyOutcome, FolderOutcome, IncomingMail, KeywordOutcome, MoveOutcome, RemoteFolder,
        RemoteMessage, RemoteSnapshot, RemoteUid, RustImap, ScanError,
    },
    sent_copy::{SentCopyError, save_sent_copy},
    submission::{
        PrepareError, PreparedMessage, SmtpSubmission, Submission, SubmissionFailure, SubmitError,
        prepare_message, prepare_message_with_attachments,
    },
    sync::{
        SyncError, sync_account, sync_account_cancellable, synchronize, synchronize_cancellable,
    },
};
