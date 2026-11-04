use std::error::Error;

/// An observed address, preserving spelling rather than merging identities.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MailboxAddress {
    pub name: Option<String>,
    pub address: String,
}

/// Owned extraction for worker handoff. This is not the complete MIME tree.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ParsedMessage {
    pub date: Option<i64>,
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub in_reply_to: Option<String>,
    pub subject: Option<String>,
    pub from: Vec<MailboxAddress>,
    pub to: Vec<MailboxAddress>,
    pub cc: Vec<MailboxAddress>,
    /// First non-attachment text/plain body, with CRLF normalized to LF.
    /// No synthesized HTML-to-text fallback.
    pub text_body: Option<String>,
    /// First non-attachment text/html body, with CRLF normalized to LF.
    /// Untrusted, unsanitized source HTML.
    pub html_body: Option<String>,
}

/// Extract decoded root headers and the first actual plain/HTML body alternatives.
///
/// Attachments and attached messages must not become the outer message's body.
/// Address groups are flattened in document order; spelling is preserved.
/// Parser recovery on malformed MIME is implementation-specific and must be
/// evaluated separately from well-formed corpus equivalence. Callers bound raw
/// input sizes and run parsing on background workers.
pub trait MimeParser {
    type Error: Error + Send + Sync + 'static;

    fn parse(&self, raw: &[u8]) -> Result<ParsedMessage, Self::Error>;
}
