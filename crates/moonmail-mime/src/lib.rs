//! MIME adapters with the same owned extraction contract.

mod mailparse_adapter;
mod normalize;
mod stalwart;

pub use {
    mailparse_adapter::{MailparseError, MailparseParser},
    stalwart::{StalwartError, StalwartParser},
};
