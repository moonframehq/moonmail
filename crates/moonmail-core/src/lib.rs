//! Backend-neutral contracts. Implementations choose their own typed errors.

mod attachments;
mod keywords;
mod mail;
mod mime;
mod predicate;
mod search;

pub use {
    attachments::{Attachment, MimeAttachments},
    keywords::canonical_keyword,
    mail::{
        BlobStore, FolderChange, MailDetail, MailPage, MailQuery, MailRepository, MailSummary,
        Mailbox, MailboxRole, PageCursor, RelatedPage, RelatedQuery, Relationship, SortField,
        ViewFilter,
    },
    mime::{MailboxAddress, MimeParser, ParsedMessage, ReplyAddresses, ReplyHeaders},
    predicate::{MailPredicate, TextField, TextTerm},
    search::{IndexChange, MessageId, SearchDocument, SearchIndex, SearchQuery},
};
