use {
    crate::MessageId,
    serde::{Deserialize, Serialize},
    std::error::Error,
};

/// Immutable exact RFC 822 bytes. A successful put must be durable before metadata
/// references it. Content hashes identify bytes, never remote message identity.
pub trait BlobStore {
    type Error: Error + Send + Sync + 'static;
    fn put(&self, bytes: &[u8]) -> Result<String, Self::Error>;
    fn get(&self, hash: &str) -> Result<Vec<u8>, Self::Error>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mailbox {
    pub id: i64,
    pub account: String,
    pub name: String,
    /// Server hierarchy separator; None means a flat namespace.
    pub delimiter: Option<String>,
    pub selectable: bool,
    pub total: u64,
    pub unread: u64,
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
    /// When present, restrict to these search hits. An empty list matches none.
    pub ids: Option<Vec<MessageId>>,
    pub sort: SortField,
    /// Text sorting is ASCII case-insensitive; equal values are ordered by ID.
    /// The chosen direction applies to both the value and the ID tie-breaker.
    pub ascending: bool,
    /// Group matching messages by account and References/In-Reply-To root key.
    /// The root message (when available) represents each group; date sorting
    /// uses its newest matching member. Pagination never splits a group.
    pub threaded: bool,
    /// Restrict to one root-key group, retaining the other query filters.
    pub thread: Option<MessageId>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MailPage {
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
    fn correspondence(&self, id: MessageId, limit: usize) -> Result<Vec<MailSummary>, Self::Error>;
    fn thread(&self, id: MessageId, limit: usize) -> Result<Vec<MailSummary>, Self::Error>;
}
