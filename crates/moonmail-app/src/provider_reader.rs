use super::*;

pub(super) fn date_label(value: Option<i64>) -> String {
    value
        .and_then(|value| time::OffsetDateTime::from_unix_timestamp(value).ok())
        .and_then(|date| {
            date.format(&time::format_description::well_known::Rfc2822)
                .ok()
        })
        .unwrap_or_default()
}

impl Mail {
    pub(super) fn reader_link_target(&self, message: MessageId, index: usize) -> Result<String> {
        anyhow::ensure!(
            self.selected
                .as_ref()
                .is_some_and(|detail| detail.summary.id == message),
            "This link belongs to a different message"
        );
        let url = self
            .reader_links
            .items
            .get(index)
            .context("Link no longer exists")?;
        crate::reader_resources::external_url(url).context("Unsupported external link")
    }

    pub(super) fn handle_reader(&mut self, action: &str, message: &Data) -> Result<()> {
        let id = MessageId(
            message
                .get("message")
                .and_then(Data::as_i64)
                .context("Missing message ID")? as u64,
        );
        if action == "reader_link" {
            let index = usize::try_from(
                message
                    .get("index")
                    .and_then(Data::as_i64)
                    .context("Missing link index")?,
            )?;
            let url = self.reader_link_target(id, index)?;
            open::that_detached(url)?;
        } else {
            let detail = self
                .selected
                .as_ref()
                .filter(|detail| detail.summary.id == id)
                .context("Select this message before loading its images")?;
            let html = detail
                .html
                .as_deref()
                .context("This message has no HTML alternative")?;
            let raw = FileBlobs::new(&self.root).get(&self.archive()?.message_blob(id)?)?;
            let images = crate::reader_resources::images(&raw, &moonmail_mime::MailparseParser)?;
            let count = images.resources.len();
            let path = self
                .reader_dir
                .as_ref()
                .context("Reader directory is missing")?
                .path()
                .join(format!("message-{}.html", id.0 % 2));
            std::fs::write(
                &path,
                crate::page::html_with_images(html, &self.preferences.reader, images.resources),
            )?;
            self.reader_revision += 1;
            self.html_url = url::Url::from_file_path(path)
                .map_err(|_| anyhow::anyhow!("Invalid reader path"))?
                .to_string();
            self.html_url.push_str(&format!(
                "?message={}&revision={}",
                id.0, self.reader_revision
            ));
            self.reader_images = format!(
                "{count} embedded images loaded · {} skipped · remote images blocked",
                images.skipped
            );
        }
        Ok(())
    }
}
