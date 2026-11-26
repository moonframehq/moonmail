use {
    super::{Email, Emails, Sync, cancel, decode},
    crate::{Error, Result, jmap::JmapError},
    moonmail_core::canonical_keyword,
    moonmail_mime::StalwartParser,
    moonmail_store::{IncomingMessage, JmapEmailHandle, JmapObservation, StoredMime},
    serde_json::Value,
    std::collections::{BTreeSet, HashMap},
};

impl<F: Fn() -> bool> Sync<'_, F> {
    pub(super) fn apply(
        &mut self,
        ids: &[String],
        value: Value,
        inventory: bool,
    ) -> Result<Vec<String>> {
        let emails: Emails = decode(value)?;
        let observed: BTreeSet<_> = emails
            .list
            .iter()
            .map(|e| &e.id)
            .chain(&emails.not_found)
            .collect();
        if observed.len() != emails.list.len() + emails.not_found.len()
            || observed != ids.iter().collect()
        {
            return Err(JmapError::Protocol.into());
        }
        if emails.list.is_empty() {
            return Ok(emails.not_found);
        }
        let folders: HashMap<_, _> = self
            .archive
            .jmap_folder_map(self.account)?
            .into_iter()
            .map(|(key, id, _)| (key, id))
            .collect();
        let keys = emails
            .list
            .iter()
            .map(|email| email.id.as_str())
            .collect::<Vec<_>>();
        let page_handles = self.archive.jmap_email_handles(self.account, &keys)?;
        let mut handles = page_handles.as_slice();
        let mut observations = Vec::new();
        let mut remaining = emails.list.as_slice();
        while !remaining.is_empty() {
            cancel(self.stop)?;
            // A large message runs alone. A window of smaller downloads shares a
            // size budget as well as respecting the server's concurrency limit.
            let mut bytes = 0_u64;
            let count = remaining
                .iter()
                .take(self.client.downloads)
                .take_while(|email| {
                    let fits = bytes == 0 || bytes.saturating_add(email.size) <= 8 * 1024 * 1024;
                    bytes = bytes.saturating_add(email.size);
                    fits
                })
                .count();
            let (window, rest) = remaining.split_at(count);
            remaining = rest;
            let (current, next_handles) = handles.split_at(count);
            handles = next_handles;
            let stored = self.prepare_window(window, current)?;
            for ((email, handle), content) in window.iter().zip(current).zip(stored) {
                cancel(self.stop)?;
                let locations: Vec<_> = email
                    .mailbox_ids
                    .keys()
                    .map(|key| folders.get(key).copied().ok_or(JmapError::Protocol))
                    .collect::<std::result::Result<_, _>>()?;
                let first = *locations.first().ok_or(JmapError::Protocol)?;
                let uid = handle.uid;
                let seen = email.keywords.contains_key("$seen");
                let flagged = email.keywords.contains_key("$flagged");
                let keywords: Vec<_> = email
                    .keywords
                    .keys()
                    .filter(|k| k.as_str() != "$seen" && k.as_str() != "$flagged")
                    .filter_map(|k| canonical_keyword(k))
                    .collect();
                let observation = if let Some(id) = handle.message {
                    Some(id)
                } else {
                    let received_at = time::OffsetDateTime::parse(
                        &email.received_at,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .map_err(|_| JmapError::Protocol)?
                    .unix_timestamp();
                    let id = self.archive.ingest_stored_with_keywords(
                        IncomingMessage {
                            account: self.account,
                            mailbox: first,
                            origin: &format!("jmap:{}", email.id),
                            epoch: 0,
                            uid: Some(uid),
                            seen,
                            starred: flagged,
                            received_at,
                            raw: content.ok_or(JmapError::Protocol)?,
                        },
                        &keywords,
                    )?;
                    // Ingest already applied the complete first occurrence and its rules.
                    (locations.len() > 1).then_some(id)
                };
                if let Some(message) = observation {
                    observations.push(JmapObservation {
                        message,
                        uid,
                        folders: locations,
                        seen,
                        flagged,
                        keywords,
                    });
                }
            }
        }
        self.archive.observe_jmap_batch(&observations)?;
        if inventory {
            self.archive.stage_jmap_ids(&keys)?;
        }
        Ok(emails.not_found)
    }

    fn prepare_window(
        &self,
        emails: &[Email],
        known: &[JmapEmailHandle],
    ) -> Result<Vec<Option<StoredMime>>> {
        let client = self.client;
        let blobs = self.blobs;
        let prepare = |email: &Email| -> Result<StoredMime> {
            if email.size > 64 * 1024 * 1024 {
                return Err(Error::IncompleteSync(1));
            }
            let raw = client.download(&email.blob_id, email.size)?;
            if raw.len() as u64 != email.size {
                return Err(JmapError::Protocol.into());
            }
            Ok(StoredMime::prepare(
                &StalwartParser::default(),
                blobs,
                &raw,
            )?)
        };
        if known.iter().filter(|id| id.message.is_none()).count() < 2 {
            return emails
                .iter()
                .zip(known)
                .map(|(email, id)| id.message.is_none().then(|| prepare(email)).transpose())
                .collect();
        }
        std::thread::scope(|scope| {
            let mut workers = Vec::with_capacity(emails.len());
            for (email, id) in emails.iter().zip(known) {
                let worker = if id.message.is_none() {
                    Some(std::thread::Builder::new().spawn_scoped(scope, || prepare(email))?)
                } else {
                    None
                };
                workers.push(worker);
            }
            workers
                .into_iter()
                .map(|worker| {
                    worker
                        .map(|w| {
                            w.join()
                                .map_err(|_| Error::Invalid("JMAP download worker panicked"))?
                        })
                        .transpose()
                })
                .collect()
        })
    }
}
