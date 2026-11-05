use {serde::Serialize, std::error::Error};

/// Metadata only. IDs are MIME-tree paths scoped to the same immutable message
/// bytes and adapter, never filesystem paths. Attached messages are opaque files;
/// their descendants are not attachments of the outer message.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub media_type: String,
    pub bytes: u64,
    pub inline: bool,
    pub content_id: Option<String>,
}

/// On-demand attachment access, separate from body/header ingestion. Inventory
/// includes explicit attachments, named parts, and non-text leaf parts (including
/// inline images). Transfer decoding must preserve binary bytes without charset
/// conversion. Implementations report limits/errors rather than truncate files.
pub trait MimeAttachments {
    type Error: Error + Send + Sync + 'static;

    fn attachments(&self, raw: &[u8]) -> Result<Vec<Attachment>, Self::Error>;
    fn attachment(&self, raw: &[u8], id: &str) -> Result<Vec<u8>, Self::Error>;
}
