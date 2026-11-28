use {
    super::*,
    crate::reader_resources::EmbeddedImages,
    std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type Resources = Arc<HashMap<String, String>>;

#[derive(Default)]
pub(super) struct ImageState {
    embedded: Resources,
    remote: Resources,
    requested: bool,
    skipped: Option<usize>,
    job: Option<ImageJob>,
    #[cfg(test)]
    pub(super) test_download: Option<fn(Vec<String>, &AtomicBool) -> EmbeddedImages>,
}

struct ImageJob {
    message: MessageId,
    cancelled: Arc<AtomicBool>,
    result: Receiver<EmbeddedImages>,
}

impl Drop for ImageJob {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}

impl ImageState {
    pub(super) fn clear_remote(&mut self) {
        if let Some(job) = &self.job {
            job.cancelled.store(true, Ordering::Relaxed);
        }
        self.remote = Default::default();
        self.skipped = None;
        self.requested = false;
    }

    pub(super) fn reset(&mut self) {
        self.clear_remote();
        self.embedded = Default::default();
    }

    pub(super) fn notice(&self, urls: &[String], offline: bool) -> serde_json::Value {
        let status = if urls.is_empty() {
            "HTML".into()
        } else if offline {
            "Remote images blocked in offline demo".into()
        } else if let Some(skipped) = self.skipped {
            if skipped == 0 {
                match self.remote.len() {
                    1 => "1 remote image loaded".into(),
                    n => format!("{n} remote images loaded"),
                }
            } else {
                format!("{} loaded · {skipped} unavailable", self.remote.len())
            }
        } else if self.requested {
            "Loading images…".into()
        } else {
            "Remote images blocked".into()
        };
        json!({"status":status, "present":!urls.is_empty(), "can_load": !offline && (!self.requested || self.skipped.is_some_and(|n| n > 0))})
    }
}

pub(super) fn date_label(value: Option<i64>) -> String {
    value
        .and_then(|value| time::OffsetDateTime::from_unix_timestamp(value).ok())
        .and_then(|date| {
            date.format(&time::format_description::well_known::Rfc2822)
                .ok()
        })
        .unwrap_or_default()
}

use crate::reader_backend::{DocumentSlot, Native, NativePages, ReaderBackend, Renderer, WebKit};

impl Mail {
    pub fn with_documents(mut self, documents: DocumentSlot) -> Self {
        self.documents = documents;
        self
    }

    pub(super) fn prepare_reader(&mut self, detail: &MailDetail) -> Result<()> {
        let revision = self.reader_revision.wrapping_add(1);

        let rich = detail.html.as_deref().map(|s| {
            crate::page::sanitize_images(
                s,
                self.image_state.embedded.clone(),
                self.image_state.remote.clone(),
            )
        });
        let font = &self.preferences.reader;
        match self.preferences.reader_renderer {
            Renderer::Native => {
                self.documents.replace(NativePages {
                    revision,
                    plain: Some(Native.plain(&detail.text, font)?.into()),
                    rich: rich
                        .as_deref()
                        .map(|s| Native.prepare(s, font))
                        .transpose()?
                        .map(Into::into),
                });
                self.plain_url.clear();
                self.html_url.clear();
            },
            Renderer::Webkit => {
                let directory = self
                    .reader_dir
                    .as_ref()
                    .context("Reader directory is missing")?
                    .path();
                let web = |alternative| WebKit {
                    directory,
                    alternative,
                    revision,
                };
                let plain_url = web("plain").plain(&detail.text, font)?;
                let html_url = rich
                    .as_deref()
                    .map(|s| web("message").prepare(s, font))
                    .transpose()?
                    .unwrap_or_default();
                self.plain_url = plain_url;
                self.html_url = html_url;
                self.documents.replace(NativePages::default());
            },
        }
        self.reader_revision = revision;
        Ok(())
    }

    fn rebuild_reader(&mut self) -> Result<()> {
        let detail = self.selected.take().context("Select a message")?;
        let result = self.prepare_reader(&detail);
        self.selected = Some(detail);
        result
    }

    pub(super) fn auto_load_images(&mut self) {
        if !self.demo && self.preferences.load_remote_images && self.image_state.skipped.is_none() {
            self.request_remote_images();
        }
    }

    fn request_remote_images(&mut self) {
        if self.reader_links.image_urls.is_empty() {
            return;
        }
        if self.image_state.skipped.is_some_and(|n| n > 0) {
            self.image_state.skipped = None;
        }
        self.image_state.requested = true;
        self.start_image_job();
    }

    fn start_image_job(&mut self) {
        let Some(detail) = &self.selected else { return };
        if !self.image_state.requested
            || self.image_state.job.is_some()
            || self.image_state.skipped.is_some()
        {
            return;
        }
        let urls = self.reader_links.image_urls.clone();
        #[cfg(test)]
        let download = self
            .image_state
            .test_download
            .unwrap_or(crate::remote_images::download);
        #[cfg(not(test))]
        let download = crate::remote_images::download;
        let (send, result) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));
        let stop = cancelled.clone();
        std::thread::spawn(move || {
            let _ = send.send(download(urls, &stop));
        });
        self.image_state.job = Some(ImageJob {
            message: detail.summary.id,
            cancelled,
            result,
        });
    }

    pub(super) fn poll_images(&mut self) -> Result<bool> {
        let Some(job) = &self.image_state.job else {
            return Ok(false);
        };
        let result = match job.result.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => return Ok(false),
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        let current = !job.cancelled.load(Ordering::Relaxed)
            && self
                .selected
                .as_ref()
                .is_some_and(|detail| detail.summary.id == job.message);
        self.image_state.job = None;
        if current {
            let result = result.unwrap_or_else(|| EmbeddedImages {
                resources: Default::default(),
                skipped: self.reader_links.image_urls.len(),
            });
            self.image_state.remote = result.resources.into();
            self.image_state.skipped = Some(result.skipped);
            self.rebuild_reader()?;
        } else {
            // At most one download worker, even when rapidly navigating messages.
            self.start_image_job();
        }
        Ok(true)
    }

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
            anyhow::ensure!(
                detail.html.is_some(),
                "This message has no HTML alternative"
            );
            if action == "reader_remote_images" {
                anyhow::ensure!(!self.demo, "The demo keeps remote images offline");
                anyhow::ensure!(
                    !self.reader_links.image_urls.is_empty(),
                    "No remote images in this message"
                );
                self.request_remote_images();
                return Ok(());
            }
            let raw = FileBlobs::new(&self.root).get(&self.archive()?.message_blob(id)?)?;
            let images = crate::reader_resources::images(&raw, &moonmail_mime::MailparseParser)?;
            let count = images.resources.len();
            self.image_state.embedded = images.resources.into();
            self.rebuild_reader()?;
            self.reader_images = format!(
                "{count} embedded images loaded · {} skipped",
                images.skipped
            );
        }
        Ok(())
    }
}
