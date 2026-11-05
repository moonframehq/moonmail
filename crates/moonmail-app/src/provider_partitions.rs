use {super::*, moonmail_store::PartitionField};

impl Mail {
    fn partition_key(&self, id: i64) -> Result<String> {
        use std::hash::{Hash, Hasher};
        let (_, predicate) = self.archive()?.resolve_search(id)?;
        let field = self.archive()?.search_partition(id)?;
        let mut hash = std::hash::DefaultHasher::new();
        serde_json::to_string(&(id, predicate, field))?.hash(&mut hash);
        Ok(format!("{:016x}", hash.finish()))
    }
    pub(super) fn partition_snapshot(&self, snapshot: &mut serde_json::Value) -> Result<()> {
        if let Some(id) = self.active_saved_search {
            snapshot["partition_value"] =
                json!(self.partition_selection.as_ref().map(|(_, value)| value));
            if self.partition_panel {
                let saved = self.archive()?.saved_search(id)?;
                let field = self.archive()?.search_partition(id)?;
                snapshot["partition_name"] = json!(saved.name);
                snapshot["partition_field"] = json!(field);
                snapshot["partition_key"] = json!(self.partition_key(id)?);
                snapshot["partition_after"] = json!(self.partition_after);
                snapshot["partition_page"] = json!(
                    field
                        .map(|field| self
                            .archive()?
                            .partition_page(field, self.partition_after.as_deref(), 100)
                            .map_err(anyhow::Error::from))
                        .transpose()?
                );
            }
        }
        Ok(())
    }
    pub(super) fn handle_partition(&mut self, action: &str, message: &Data) -> Result<()> {
        let text = |key| message.get(key).and_then(Data::as_str).unwrap_or_default();
        let id = message
            .get("id")
            .and_then(Data::as_i64)
            .context("Choose a saved search")?;
        if action == "partition_open" {
            let (saved, predicate) = self.archive()?.resolve_search(id)?;
            self.set_search(saved.source, Some(predicate), true)?;
            self.active_saved_search = Some(id);
            self.partition_panel = true;
            return Ok(());
        }
        anyhow::ensure!(
            self.partition_panel
                && self.active_saved_search == Some(id)
                && text("key") == self.partition_key(id)?,
            "Partition source changed; reopen its groups"
        );
        match action {
            "partition_field" => {
                let field = if text("field") == "none" {
                    None
                } else {
                    Some(PartitionField::parse(text("field"))?)
                };
                self.archive()?.set_search_partition(id, field)?;
                self.partition_after = None;
            },
            "partition_first" => self.partition_after = None,
            "partition_next" | "partition_choose" => {
                anyhow::ensure!(
                    message.get("after").and_then(Data::as_str) == self.partition_after.as_deref(),
                    "Partition page changed"
                );
                let field = self
                    .archive()?
                    .search_partition(id)?
                    .context("Choose a partition field")?;
                let page =
                    self.archive()?
                        .partition_page(field, self.partition_after.as_deref(), 100)?;
                if action == "partition_next" {
                    anyhow::ensure!(
                        page.next.as_deref() == Some(text("value")),
                        "Partition page changed"
                    );
                    self.partition_after = page.next;
                } else {
                    let value = text("value").to_owned();
                    anyhow::ensure!(
                        page.rows.iter().any(|bucket| bucket.value == value),
                        "Partition no longer exists on this page"
                    );
                    let (saved, predicate) = self.archive()?.resolve_search(id)?;
                    let predicate =
                        moonmail_core::MailPredicate::All(vec![predicate, field.predicate(&value)]);
                    self.set_search(saved.source, Some(predicate), true)?;
                    self.active_saved_search = Some(id);
                    self.partition_selection = Some((field, value));
                }
            },
            _ => bail!("Unknown partition action"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tests::data, *};
    #[test]
    fn partition_actions_bind_source_field_page_and_keep_exact_filter_on_refresh() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut archive = Archive::open(dir.path())?;
        moonmail_store::seed_demo(&mut archive, &FileBlobs::new(dir.path()))?;
        let id = archive.save_search(
            "All demo",
            "account:you@example.com",
            &moonmail_core::MailPredicate::Account("you@example.com".into()),
        )?;
        drop(archive);
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        mail.handle(data(json!({"action":"partition_open","id":id})))?;
        let key = mail.snapshot()?["partition_key"].clone();
        mail.handle(data(
            json!({"action":"partition_field","id":id,"key":key,"field":"sender"}),
        ))?;
        let snapshot = mail.snapshot()?;
        let key = snapshot["partition_key"].clone();
        let value = snapshot["partition_page"]["rows"][0]["value"].clone();
        assert!(
            mail.handle(data(
                json!({"action":"partition_choose","id":id,"key":key,"after":"stale","value":value})
            ))
            .is_err()
        );
        mail.handle(data(
            json!({"action":"partition_choose","id":id,"key":key,"after":null,"value":value}),
        ))?;
        let count = mail.page.rows.len();
        assert!(count > 0 && count < 72);
        mail.refresh()?;
        assert_eq!(mail.page.rows.len(), count);
        assert_eq!(mail.snapshot()?["partition_value"], value);
        mail.archive_mut()?.save_search(
            "All demo",
            "is:unread",
            &moonmail_core::MailPredicate::Unread,
        )?;
        mail.refresh()?;
        assert_eq!(mail.snapshot()?["partition_value"], value);
        assert!(mail.page.rows.iter().all(|r| r.unread));
        mail.handle(data(json!({"action":"partition_open","id":id})))?;
        assert!(
            mail.handle(data(
                json!({"action":"partition_field","id":id,"key":key,"field":"keyword"})
            ))
            .is_err(),
            "old parent predicate key is stale"
        );
        mail.handle(data(json!({"action":"search_open","id":id})))?;
        assert!(mail.snapshot()?["partition_value"].is_null());
        assert_eq!(
            mail.archive()?.search_partition(id)?,
            Some(PartitionField::Sender)
        );
        Ok(())
    }
}
