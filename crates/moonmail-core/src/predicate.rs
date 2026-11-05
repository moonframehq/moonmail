use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextField {
    Any,
    Subject,
    Body,
    From,
    To,
    Cc,
}

/// A literal token phrase in one indexed field; backend query syntax is never
/// interpolated from `value`. Analyzer semantics match SearchIndex::search.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TextTerm {
    pub field: TextField,
    pub value: String,
}

/// Shared direct/saved-query semantics. Live/folder/list scope always intersects
/// this predicate. NOT complements within that scope. Metadata predicates remain
/// live after preparation; text terms use one committed projection snapshot.
/// Account/mailbox strings use exact stored equality. Dates compare summary sort
/// timestamps, after inclusively and before exclusively (UTC Unix seconds).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailPredicate {
    Text(TextTerm),
    Account(String),
    Mailbox(String),
    /// Exact parsed address: preserve local-part case, fold ASCII domain case.
    SenderAddress(String),
    /// Exact To or Cc address; never infer undisclosed Bcc recipients.
    RecipientAddress(String),
    /// Case-insensitive IMAP atom on any live occurrence.
    Keyword(String),
    /// Exact account-local friendly label on any live occurrence.
    Tag(String),
    Unread,
    Starred,
    After(i64),
    Before(i64),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
}
