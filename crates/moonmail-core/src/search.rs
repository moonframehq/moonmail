use std::{error::Error, num::NonZeroUsize};

/// Application-local identity, not an IMAP UID or an RFC Message-ID.
#[derive(
    Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, serde::Serialize, serde::Deserialize,
)]
pub struct MessageId(pub u64);

/// Prepared searchable text; extraction and HTML conversion happen upstream.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchDocument {
    pub subject: String,
    pub body: String,
    pub from: String,
    pub to: String,
    pub cc: String,
}

#[derive(Clone, Copy, Debug)]
pub enum IndexChange<'a> {
    Upsert(MessageId, &'a SearchDocument),
    Delete(MessageId),
}

/// All nonempty phrases must match somewhere in the document. Each phrase is
/// literal text, never backend query syntax. An empty query matches nothing.
///
/// Case/diacritic folding and token boundaries belong to the backend's documented
/// analyzer. Compare engines only on compatible analyzers/corpora. Exact address
/// equality and Correspondence use metadata indexes, not this text search API.
#[derive(Clone, Copy, Debug)]
pub struct SearchQuery<'a> {
    pub phrases: &'a [&'a str],
    pub limit: NonZeroUsize,
}

/// A rebuildable search projection, not authoritative message storage.
///
/// Mutations apply in order, atomically: an error must expose none of the batch.
/// Successful mutations are visible to subsequent searches, including deletion.
/// Missing deletes are idempotent. Search returns unique IDs, best match first,
/// ties ordered by ascending ID, with at most `limit` results. Relevance scores
/// are engine-specific and deliberately not exposed as portable values.
///
/// Persistent adapters document their commit durability. Implementations must not
/// claim an asynchronous indexing queue has committed before it becomes visible.
pub trait SearchIndex {
    type Error: Error + Send + Sync + 'static;

    fn apply(&mut self, changes: &[IndexChange<'_>]) -> Result<(), Self::Error>;

    fn search(&self, query: SearchQuery<'_>) -> Result<Vec<MessageId>, Self::Error>;
}
