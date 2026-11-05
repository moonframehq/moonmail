use {
    serde::{Deserialize, Serialize},
    std::error::Error,
};

/// An observed address, preserving spelling rather than merging identities.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
    pub reply_to: Vec<MailboxAddress>,
    pub to: Vec<MailboxAddress>,
    pub cc: Vec<MailboxAddress>,
    /// First non-attachment text/plain body, with CRLF normalized to LF.
    /// No synthesized HTML-to-text fallback.
    pub text_body: Option<String>,
    /// First non-attachment text/html body, with CRLF normalized to LF.
    /// Untrusted, unsanitized source HTML.
    pub html_body: Option<String>,
}

/// Original root headers used when composing a reply. References retain their
/// source order; In-Reply-To is the fallback when no References were supplied.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReplyHeaders {
    pub message_id: Option<String>,
    pub references: Vec<String>,
    pub in_reply_to: Option<String>,
    pub addresses: ReplyAddresses,
}

/// Original recipient roles for reply construction. Bcc is deliberately absent:
/// it must never be redisclosed by Reply-All, including replies to sent mail.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReplyAddresses {
    pub from: Vec<MailboxAddress>,
    pub reply_to: Vec<MailboxAddress>,
    pub to: Vec<MailboxAddress>,
    pub cc: Vec<MailboxAddress>,
}

impl From<&ParsedMessage> for ReplyAddresses {
    fn from(message: &ParsedMessage) -> Self {
        Self {
            from: message.from.clone(),
            reply_to: message.reply_to.clone(),
            to: message.to.clone(),
            cc: message.cc.clone(),
        }
    }
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
