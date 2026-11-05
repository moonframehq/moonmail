use {super::*, moonmail_core::MimeAttachments, std::io::Read};

impl Mail {
    pub(super) fn copy_forward_attachments(&self, draft: &mut Draft, id: MessageId) -> Result<()> {
        let raw = FileBlobs::new(&self.root).get(&self.archive()?.message_blob(id)?)?;
        let parser = moonmail_mime::MailparseParser;
        let attachments = parser.attachments(&raw)?;
        anyhow::ensure!(
            attachments.len() <= moonmail_store::MAX_DRAFT_ATTACHMENTS
                && attachments.iter().map(|item| item.bytes).sum::<u64>()
                    <= moonmail_store::MAX_ATTACHMENT_BYTES,
            "Forward exceeds 16 files or 25 MiB; use Mail → Forward text only instead"
        );
        let blobs = FileBlobs::attachments(&self.root);
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

    pub(super) fn handle_attachment(&mut self, action: &str, message: &Data) -> Result<()> {
        if message.get("cancelled").and_then(Data::as_bool) == Some(true) {
            return Ok(());
        }
        let text = |key| message.get(key).and_then(Data::as_str).unwrap_or_default();
        match action {
            "attachments" => {
                let id = MessageId(
                    message
                        .get("message")
                        .and_then(Data::as_i64)
                        .context("Missing message ID")? as u64,
                );
                anyhow::ensure!(
                    self.selected
                        .as_ref()
                        .is_some_and(|detail| detail.summary.id == id),
                    "Select this message before inspecting its attachments"
                );
                self.attachments.clear();
                self.attachment_message = None;
                let blob = self.archive()?.message_blob(id)?;
                let raw = FileBlobs::new(&self.root).get(&blob)?;
                let attachments = moonmail_mime::MailparseParser.attachments(&raw)?;
                self.attachments = attachments;
                self.attachment_message = Some(id);
            },
            "messages_save" => {
                let ids = self.move_selection();
                self.validate_move_choice(message, &ids)?;
                let Some(Data::List(paths)) = message.get("paths") else {
                    bail!("Choose an export folder");
                };
                anyhow::ensure!(paths.len() == 1, "Choose exactly one export folder");
                let path = paths[0].as_str().context("Invalid export folder")?;
                let directory = crate::file_export::messages(
                    self.archive()?,
                    &FileBlobs::new(&self.root),
                    &ids,
                    &PathBuf::from(path),
                )?;
                self.status = format!(
                    "Exported {} original message(s) to {}",
                    ids.len(),
                    directory.display()
                );
            },
            "message_save" => {
                let id = MessageId(
                    message
                        .get("message")
                        .and_then(Data::as_i64)
                        .filter(|id| *id > 0)
                        .context("Missing message ID")? as u64,
                );
                anyhow::ensure!(
                    self.selected
                        .as_ref()
                        .is_some_and(|detail| detail.summary.id == id),
                    "This save panel belongs to another message; reopen Save message"
                );
                let path = PathBuf::from(text("path"));
                crate::file_export::message(
                    self.archive()?,
                    &FileBlobs::new(&self.root),
                    id,
                    &path,
                )?;
                self.status = format!("Saved original message to {}", path.display());
            },
            "attachment_save" => {
                let id = MessageId(
                    message
                        .get("message")
                        .and_then(Data::as_i64)
                        .context("Missing message ID")? as u64,
                );
                anyhow::ensure!(
                    self.attachment_message == Some(id)
                        && self.attachments.iter().any(|item| item.id == text("part")),
                    "This attachment view is stale; reopen the message attachments"
                );
                let path = PathBuf::from(text("path"));
                anyhow::ensure!(
                    path.is_absolute(),
                    "Choose an absolute destination with Save As"
                );
                let blob = self.archive()?.message_blob(id)?;
                let raw = FileBlobs::new(&self.root).get(&blob)?;
                let bytes = moonmail_mime::MailparseParser.attachment(&raw, text("part"))?;
                crate::file_export::save_new(&path, &bytes)?;
                self.status = format!("Saved attachment to {}", path.display());
            },
            "draft_attach" | "draft_detach" => {
                let mut draft = self.draft.clone().context("Open a draft first")?;
                anyhow::ensure!(
                    draft.id == text("id"),
                    "The file picker belongs to a different draft; reopen the intended draft"
                );
                if action == "draft_attach" {
                    let Some(Data::List(paths)) = message.get("paths") else {
                        bail!("Choose files to attach");
                    };
                    anyhow::ensure!(
                        draft.attachments.len() + paths.len()
                            <= moonmail_store::MAX_DRAFT_ATTACHMENTS,
                        "A draft supports at most 16 attachments"
                    );
                    draft.validate_attachments()?;
                    let mut remaining = moonmail_store::MAX_ATTACHMENT_BYTES
                        - draft.attachments.iter().map(|item| item.bytes).sum::<u64>();
                    let blobs = FileBlobs::attachments(&self.root);
                    for path in paths {
                        let path = PathBuf::from(path.as_str().context("Invalid attachment path")?);
                        anyhow::ensure!(path.is_absolute(), "Choose an absolute attachment path");
                        anyhow::ensure!(
                            std::fs::metadata(&path)?.is_file(),
                            "Attachments must be regular files"
                        );
                        let file = std::fs::File::open(&path)?;
                        let metadata = file.metadata()?;
                        anyhow::ensure!(metadata.is_file(), "Attachments must be regular files");
                        anyhow::ensure!(
                            metadata.len() <= remaining,
                            "Attachments exceed 25 MiB total"
                        );
                        let mut bytes = Vec::new();
                        file.take(remaining + 1).read_to_end(&mut bytes)?;
                        anyhow::ensure!(
                            bytes.len() as u64 <= remaining,
                            "Attachments exceed 25 MiB total"
                        );
                        remaining -= bytes.len() as u64;
                        let name = moonmail_mime::attachment_name(
                            &path
                                .file_name()
                                .context("Missing filename")?
                                .to_string_lossy(),
                        );
                        let blob = blobs.put(&bytes)?;
                        draft.attachments.push(moonmail_store::DraftAttachment {
                            id: uuid::Uuid::new_v4().to_string(),
                            name,
                            blob,
                            bytes: bytes.len() as u64,
                        });
                    }
                } else {
                    let part = text("part");
                    anyhow::ensure!(
                        draft.attachments.iter().any(|item| item.id == part),
                        "Attachment no longer exists in this draft"
                    );
                    draft.attachments.retain(|item| item.id != part);
                }
                // One durable edit after every selected file has been copied.
                // Failed batches may leave unreferenced blobs, but no partial draft.
                self.archive()?.save_draft(&draft)?;
                self.draft = Some(draft);
                self.draft_before = None;
            },
            _ => {},
        }
        Ok(())
    }
}
