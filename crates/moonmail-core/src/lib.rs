//! Backend-neutral contracts. Implementations choose their own typed errors.

mod mail;
mod mime;
mod search;

pub use {
    mail::{
        BlobStore, MailDetail, MailPage, MailQuery, MailRepository, MailSummary, Mailbox,
        PageCursor, SortField, ViewFilter,
    },
    mime::{MailboxAddress, MimeParser, ParsedMessage},
    search::{IndexChange, MessageId, SearchDocument, SearchIndex, SearchQuery},
};
