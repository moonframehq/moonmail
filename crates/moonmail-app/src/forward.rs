use {
    anyhow::Result,
    moonmail_core::{BlobStore, MessageId, MimeAttachments},
    moonmail_store::{Archive, Draft, FileBlobs},
    std::path::Path,
};

/// Copy a message's attachments into a forward draft, within draft limits.
pub fn copy_attachments(
    root: &Path,
    archive: &Archive,
    draft: &mut Draft,
    id: MessageId,
) -> Result<()> {
    let raw = FileBlobs::new(root).get(&archive.message_blob(id)?)?;
    let parser = moonmail_mime::MailparseParser;
    let attachments = parser.attachments(&raw)?;
    anyhow::ensure!(
        attachments.len() <= moonmail_store::MAX_DRAFT_ATTACHMENTS
            && attachments.iter().map(|item| item.bytes).sum::<u64>()
                <= moonmail_store::MAX_ATTACHMENT_BYTES,
        "Forward exceeds 16 files or 25 MiB; use Mail → Forward text only instead"
    );
    let blobs = FileBlobs::attachments(root);
    for item in attachments {
        let bytes = parser.attachment(&raw, &item.id)?;
        draft.attachments.push(moonmail_store::DraftAttachment {
            id: uuid::Uuid::new_v4().to_string(),
            name: item.name,
            blob: blobs.put(&bytes)?,
            bytes: bytes.len() as u64,
        });
    }
    Ok(())
}
