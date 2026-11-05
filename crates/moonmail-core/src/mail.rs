use {
    crate::MessageId,
    serde::{Deserialize, Serialize},
    std::error::Error,
};

/// Immutable bytes (original RFC 822 or a draft attachment). A successful put must be durable before metadata
/// references it. Content hashes identify bytes, never remote message identity.
pub trait BlobStore {
    type Error: Error + Send + Sync + 'static;
    fn put(&self, bytes: &[u8]) -> Result<String, Self::Error>;
    fn get(&self, hash: &str) -> Result<Vec<u8>, Self::Error>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mailbox {
    /// Last durably observed UIDVALIDITY; zero for an unscanned/local folder.
    pub epoch: u32,
    pub id: i64,
    pub account: String,
    pub name: String,
    /// Server hierarchy separator; None means a flat namespace.
    pub delimiter: Option<String>,
    pub selectable: bool,
    /// Explicit user override, otherwise server SPECIAL-USE (INBOX is intrinsic).
    pub role: Option<MailboxRole>,
    pub total: u64,
    pub unread: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxRole {
    Inbox,
    Archive,
    Sent,
    Drafts,
    Junk,
    Trash,
    All,
    Flagged,
}

impl MailboxRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Archive => "archive",
            Self::Sent => "sent",
            Self::Drafts => "drafts",
            Self::Junk => "junk",
            Self::Trash => "trash",
            Self::All => "all",
            Self::Flagged => "flagged",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "inbox" => Self::Inbox,
            "archive" => Self::Archive,
            "sent" => Self::Sent,
            "drafts" => Self::Drafts,
            "junk" => Self::Junk,
            "trash" => Self::Trash,
            "all" => Self::All,
            "flagged" => Self::Flagged,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailSummary {
    pub id: MessageId,
    pub subject: String,
    pub sender: String,
    pub address: String,
    pub recipients: String,
    pub date: i64,
    pub date_label: String,
    pub snippet: String,
    /// True when any represented matching message has an unread occurrence.
    pub unread: bool,
    pub starred: bool,
    /// Number of matching messages represented by a collapsed listing row.
    pub thread_count: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MailDetail {
    pub summary: MailSummary,
    pub text: String,
    pub html: Option<String>,
    pub blob: String,
    pub rfc_message_id: Option<String>,
    /// Parsed Date header and independent source timestamp (IMAP INTERNALDATE).
    /// Legacy archives may lack both; the existing summary sort date is retained.
    pub header_date: Option<i64>,
    pub received_at: Option<i64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ViewFilter {
    #[default]
    All,
    Unread,
    Starred,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
/// An opaque keyset position. Reuse only with the same query, grouping, sort
/// field and direction. Clear it whenever any of those settings changes.
pub struct PageCursor {
    pub date: i64,
    pub id: MessageId,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Date,
    Sender,
    Subject,
}

#[derive(Clone, Debug, Default)]
pub struct MailQuery {
    pub mailbox: Option<i64>,
    pub filter: ViewFilter,
    pub before: Option<PageCursor>,
    /// Optional explicit ID restriction, independent of the search snapshot.
    /// An empty list matches none. Adapters may bound this caller-owned list.
    pub ids: Option<Vec<MessageId>>,
    /// Restrict to the repository's prepared search-match snapshot. Preparation
    /// is an explicit adapter operation; requesting an unprepared set is an
    /// error. Scope and grouping apply before page limits, without a hit cap.
    pub search_matches: bool,
    pub sort: SortField,
    /// Text sorting is ASCII case-insensitive; equal values are ordered by ID.
    /// The chosen direction applies to both the value and the ID tie-breaker.
    pub ascending: bool,
    /// Group matching messages by account-scoped reference component. Prefer
    /// members without parent links, then date/ID; date sorting uses newest
    /// matching activity. Pagination never splits a group.
    pub threaded: bool,
    /// Keep only rows with unread matching members, after grouping. Unlike
    /// ViewFilter::Unread, this preserves representatives and group sort keys,
    /// so navigation cursors can be reused across this restriction.
    pub unread_only: bool,
    /// Restrict to one reference component, retaining the other query filters.
    pub thread: Option<MessageId>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MailPage {
    pub rows: Vec<MailSummary>,
    pub next: Option<PageCursor>,
    /// Bounds of the returned page, including a terminal page. Grouped date
    /// cursors use group activity, not the representative message's Date.
    /// These permit reversing keyset direction without guessing from summaries.
    pub first: Option<PageCursor>,
    pub last: Option<PageCursor>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relationship {
    Correspondence,
    Thread,
}

/// A single server namespace mutation. Rename includes descendants according to
/// the server's hierarchy delimiter; delete removes only the named mailbox.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FolderChange {
    Create { name: String },
    Rename { source: String, destination: String },
    Delete { name: String },
}

#[derive(Clone, Debug)]
pub struct RelatedQuery {
    pub anchor: MessageId,
    pub relationship: Relationship,
    pub sort: SortField,
    pub ascending: bool,
    pub before: Option<PageCursor>,
}

#[derive(Clone, Debug, Serialize)]
pub struct RelatedPage {
    /// Pinned separately and excluded from rows/cursors, even when non-live.
    pub anchor: MailSummary,
    pub rows: Vec<MailSummary>,
    pub next: Option<PageCursor>,
}

/// Interactive, bounded read boundary. Writes remain explicit transactions in the
/// repository adapter; UI code must not independently advance protocol cursors.
pub trait MailRepository {
    type Error: Error + Send + Sync + 'static;
    fn mailboxes(&self) -> Result<Vec<Mailbox>, Self::Error>;
    fn page(&self, query: &MailQuery, limit: usize) -> Result<MailPage, Self::Error>;
    fn detail(&self, id: MessageId) -> Result<MailDetail, Self::Error>;
    /// Newest related messages plus the anchor, ordered by descending date/ID.
    /// The clamped limit includes the anchor: even an old, non-live or recipient-
    /// less anchor is retained. A limit of one returns only the anchor.
    fn correspondence(&self, id: MessageId, limit: usize) -> Result<Vec<MailSummary>, Self::Error>;
    fn thread(&self, id: MessageId, limit: usize) -> Result<Vec<MailSummary>, Self::Error>;
    /// Complete keyset navigation, metadata only. Correspondence matches exact
    /// bidirectional From/To-or-Cc pairs across live messages, excluding any
    /// occurrence in Junk/Trash (explicit roles override name fallbacks). Thread
    /// uses the account-scoped root-key group. Sort semantics match `page`.
    /// The limit bounds non-anchor rows. Reuse cursors only for the same anchor,
    /// relationship, sort and direction; restart to include newer arrivals.
    fn related_page(&self, query: &RelatedQuery, limit: usize) -> Result<RelatedPage, Self::Error>;
}
