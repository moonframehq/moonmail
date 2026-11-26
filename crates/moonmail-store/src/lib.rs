//! Durable local archive and rebuildable projections.
mod archive;
mod blobs;
mod correspondents;
mod demo;
mod drafts;
mod error;
mod folder_operations;
mod gmail;
mod identities;
mod keywords;
mod mailboxes;
mod moves;
mod operations;
mod outbox;
mod partitions;
mod predicates;
mod queries;
mod relationships;
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
    archive::{Archive, IncomingMessage, IndexWork},
    blobs::{BlobError, FileBlobs},
    correspondents::Correspondent,
    demo::seed_demo,
    drafts::{
        Draft, DraftAttachment, DraftCursor, DraftPage, DraftSummary, MAX_ATTACHMENT_BYTES,
        MAX_DRAFT_ATTACHMENTS,
    },
    error::{Error, IngestError, Result, SearchError},
    folder_operations::{FolderEdit, FolderMember, FolderOperation, FolderOperationSummary},
    gmail::valid_gmail_label,
    identities::Identity,
    keywords::{KeywordLabel, KeywordOperation},
    moves::{MoveOperation, MoveTarget},
    operations::PendingFlag,
    outbox::{OutboxItem, OutboxPage},
    partitions::{PartitionBucket, PartitionField, PartitionPage},
    rules::{KeywordRule, KeywordRuleRun},
    saved_searches::SavedSearch,
    sync_inventory::{GmailMetadata, SyncMessage},
};
