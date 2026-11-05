use super::*;

impl Mail {
    pub(super) fn set_search(
        &mut self,
        source: String,
        saved: Option<moonmail_core::MailPredicate>,
        reset_scope: bool,
    ) -> Result<()> {
        if !source.trim().is_empty() {
            let predicate = match &saved {
                Some(predicate) => predicate.clone(),
                None => moonmail_search::parse_query(&source)?,
            };
            self.archive
                .as_mut()
                .context("Archive is starting")?
                .prepare_predicate(
                    self.index.as_ref().context("Index is starting")?,
                    &predicate,
                )?;
        }
        self.saved_predicate = saved;
        self.active_saved_search = None;
        self.partition_selection = None;
        self.partition_after = None;
        self.partition_panel = false;
        self.prepared_search = Some(source.clone());
        self.search = source;
        if reset_scope {
            self.query.mailbox = None;
            self.query.filter = ViewFilter::All;
        }
        self.query.before = None;
        self.expanded.clear();
        self.checked.clear();
        self.refresh()
    }
    pub(super) fn handle_search(&mut self, action: &str, message: &Data) -> Result<()> {
        let id = || {
            message
                .get("id")
                .and_then(Data::as_i64)
                .context("Choose a saved search")
        };
        match action {
            "search_save" => {
                let name = message
                    .get("name")
                    .and_then(Data::as_str)
                    .context("Name the search")?
                    .trim();
                let source = self.search.clone();
                let predicate = match self.active_saved_search {
                    Some(id) => self.archive()?.saved_search(id)?.predicate,
                    None => moonmail_search::parse_query(&source)?,
                };
                let sources = match message.get("sources") {
                    None => Vec::new(),
                    Some(Data::List(ids)) => ids
                        .iter()
                        .map(|id| id.as_i64().context("Invalid saved search source"))
                        .collect::<Result<Vec<_>>>()?,
                    _ => anyhow::bail!("Invalid saved search sources"),
                };
                self.archive_mut()?
                    .save_search_from(name, &source, &predicate, &sources)?;
                self.refresh()?;
                self.status="Query saved. Selected sources are combined with OR, then filtered by this query.".into();
            },
            "search_open" => {
                let id = id()?;
                let (saved, predicate) = self.archive()?.resolve_search(id)?;
                self.set_search(saved.source, Some(predicate), true)?;
                self.active_saved_search = Some(id);
            },
            "search_delete" => {
                let id = id()?;
                self.archive()?.delete_search(id)?;
                if self.active_saved_search == Some(id) {
                    self.set_search(String::new(), None, false)?;
                }
            },
            _ => {},
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tests::data, *};
    #[test]
    fn saved_query_opens_its_predicate_and_live_flags_change_results_without_reindexing()
    -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut archive = Archive::open(dir.path())?;
        let mailbox = archive.mailbox("you@example.com", "INBOX")?;
        for n in 0..3 {
            archive.ingest(&moonmail_mime::StalwartParser::default(),&FileBlobs::new(dir.path()),moonmail_store::IncomingMessage {account:"you@example.com",mailbox,origin:&n.to_string(),epoch:0,uid:None,seen:n==2,starred:false,received_at:n,raw:b"From: Alice <alice@example.org>\r\nSubject: Review notes\r\n\r\nSynthetic"})?;
        }
        drop(archive);
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        while mail.index_pending()? {}
        let source = "subject:\"Review notes\" is:unread";
        mail.handle(data(json!({"action":"search","value":source})))?;
        assert_eq!(mail.page.rows.len(), 2);
        mail.handle(data(
            json!({"action":"search_save","name":"Unread reviews"}),
        ))?;
        let saved = mail.archive()?.saved_searches()?.remove(0);
        assert!(
            mail.handle(data(json!({"action":"search","value":"subject:"})))
                .is_err()
        );
        assert_eq!(
            mail.search, source,
            "malformed input must not replace the active query"
        );
        // Valid syntax can still fail during preparation. Neither compiled-query
        // rejection nor an unavailable index may replace the active view.
        let prior_ids: Vec<_> = mail.page.rows.iter().map(|row| row.id).collect();
        let excessive = vec!["review"; 65].join(" ");
        assert!(moonmail_search::parse_query(&excessive).is_ok());
        assert!(
            mail.handle(data(json!({"action":"search","value":excessive})))
                .is_err()
        );
        assert_eq!(mail.search, source);
        let index = mail.index.take();
        assert!(
            mail.handle(data(json!({"action":"search_open","id":saved.id})))
                .is_err()
        );
        mail.index = index;
        assert_eq!(mail.search, source);
        mail.refresh()?;
        assert_eq!(
            mail.page.rows.iter().map(|row| row.id).collect::<Vec<_>>(),
            prior_ids
        );
        mail.handle(data(json!({"action":"search","value":"body:absent"})))?;
        assert!(mail.page.rows.is_empty());
        mail.query.mailbox = Some(-1);
        mail.handle(data(json!({"action":"search_open","id":saved.id})))?;
        assert_eq!(mail.page.rows.len(), 2);
        assert!(mail.query.mailbox.is_none());
        let id = mail.page.rows[0].id;
        mail.archive_mut()?.set_flag(id, "seen", true)?;
        mail.refresh()?;
        assert_eq!(mail.page.rows.len(), 1);
        let parent = mail.archive_mut()?.save_search(
            "Parent",
            "from:alice",
            &moonmail_search::parse_query("from:alice")?,
        )?;
        let child = mail.archive_mut()?.save_search_from(
            "Child",
            source,
            &moonmail_search::parse_query(source)?,
            &[parent],
        )?;
        mail.handle(data(json!({"action":"search_open","id":child})))?;
        assert_eq!(mail.page.rows.len(), 1);
        // A separate worker may edit the source. Refresh resolves the source
        // graph again, even when the child's displayed query text is unchanged.
        let mut other = Archive::open(dir.path())?;
        other.save_search(
            "Parent",
            "from:absent",
            &moonmail_search::parse_query("from:absent")?,
        )?;
        mail.refresh()?;
        assert!(mail.page.rows.is_empty());
        assert_eq!(mail.active_saved_search, Some(child));
        mail.handle(data(json!({"action":"search","value":source})))?;
        assert_eq!(mail.page.rows.len(), 1);
        assert!(mail.active_saved_search.is_none());
        mail.handle(data(json!({"action":"search_delete","id":saved.id})))?;
        assert!(
            mail.handle(data(json!({"action":"search_open","id":saved.id})))
                .is_err()
        );
        Ok(())
    }
}
