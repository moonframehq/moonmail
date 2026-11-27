//! Durable local archive and rebuildable projections.
mod archive;
mod blobs;
mod correspondents;
mod demo;
mod drafts;
mod error;
mod expunge;
mod folder_operations;
mod gmail;
mod identities;
mod keywords;
mod mailboxes;
mod maildir;
mod moves;
mod msgvault;
mod operations;
mod outbox;
mod partitions;
mod predicates;
mod queries;
mod relationships;
mod remote_drafts;
mod replies;
mod rules;
mod saved_searches;
mod search;
mod search_sources;
mod sent;
mod sent_copy;
mod sync_inventory;
mod thread_graph;

pub use {
    archive::{Archive, IncomingMessage, IndexWork, StoredMime},
    blobs::{BlobError, FileBlobs, content_digest},
    correspondents::Correspondent,
    demo::seed_demo,
    drafts::{
        Draft, DraftAttachment, DraftCursor, DraftPage, DraftSummary, MAX_ATTACHMENT_BYTES,
        MAX_DRAFT_ATTACHMENTS,
    },
    error::{Error, IngestError, Result, SearchError},
    expunge::{DeletionTarget, RemoteOperation},
    folder_operations::{FolderEdit, FolderMember, FolderOperation, FolderOperationSummary},
    gmail::valid_gmail_label,
    identities::Identity,
    keywords::{KeywordLabel, KeywordOperation},
    maildir::{MaildirAccount, MaildirFile, MaildirLocation},
    moves::{MoveOperation, MoveTarget},
    msgvault::{MsgvaultAccount, MsgvaultChange, MsgvaultState},
    operations::PendingFlag,
    outbox::{OutboxItem, OutboxPage},
    partitions::{PartitionBucket, PartitionField, PartitionPage},
    remote_drafts::RemoteDraftSource,
    rules::{KeywordRule, KeywordRuleRun},
    saved_searches::SavedSearch,
    sync_inventory::{GmailMetadata, InventoryKind, SyncCheckpoint, SyncFlags, SyncMessage},
};
mod jmap;
mod jmap_emails;
mod jmap_flags;
pub use {
    jmap::{JmapFolder, JmapObservation},
    jmap_emails::JmapEmailHandle,
    jmap_flags::JmapFlag,
};

mod jmap_deletions;
pub use jmap_deletions::JmapDeletion;
