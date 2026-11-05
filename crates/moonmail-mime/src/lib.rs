//! MIME adapters with the same owned extraction contract.

mod attachments;
mod mailparse_adapter;
mod normalize;
mod stalwart;

pub use {
    attachments::{AttachmentError, attachment_name},
    mailparse_adapter::{MailparseError, MailparseParser},
    stalwart::{StalwartError, StalwartParser},
};
