use super::*;

impl Mail {
    pub(super) fn expanded_query(&self, id: MessageId) -> MailQuery {
        let mut query = self.query.clone();
        query.threaded = false;
        query.thread = Some(id);
        query.before = self.expanded.get(&id).cloned().flatten();
        query.sort = SortField::Date;
        query.ascending = true;
        query
    }

    pub(super) fn handle_thread_page(&mut self, action: &str, message: &Data) -> Result<()> {
        let id = MessageId(
            message
                .get("id")
                .and_then(Data::as_i64)
                .context("Missing expanded thread")? as u64,
        );
        anyhow::ensure!(
            self.query.threaded
                && self.expanded.contains_key(&id)
                && self.page.rows.iter().any(|row| row.id == id),
            "This expanded thread is no longer visible"
        );
        let query = self.expanded_query(id);
        anyhow::ensure!(
            message.get("cursor").and_then(Data::as_i64)
                == Some(query.before.as_ref().map_or(0, |cursor| cursor.id.0 as i64)),
            "This expanded-thread page is stale"
        );
        if action == "thread_first" {
            self.expanded.insert(id, None);
        } else if let Some(next) = self.archive()?.page(&self.expanded_query(id), 100)?.next {
            self.expanded.insert(id, Some(next));
        }
        self.checked.clear();
        Ok(())
    }

    pub(super) fn related_query(&self) -> Option<RelatedQuery> {
        self.anchor.map(|anchor| RelatedQuery {
            anchor,
            relationship: self.relationship,
            sort: self.related_sort,
            ascending: self.related_ascending,
            before: self.related_before.clone(),
        })
    }

    pub(super) fn handle_relationship(&mut self, action: &str, message: &Data) -> Result<()> {
        let anchor = message
            .get("anchor")
            .and_then(Data::as_i64)
            .map(|id| MessageId(id as u64));
        anyhow::ensure!(
            anchor.is_some() && anchor == self.anchor,
            "This related-message view is stale"
        );
        match action {
            "related_first" => self.related_before = None,
            "related_next" => {
                if let Some(query) = self.related_query()
                    && let Some(next) = self.archive()?.related_page(&query, 50)?.next
                {
                    self.related_before = Some(next);
                }
            },
            "related_sort" => {
                let sort = match message.get("field").and_then(Data::as_str) {
                    Some("date") => SortField::Date,
                    Some("sender") => SortField::Sender,
                    Some("subject") => SortField::Subject,
                    _ => bail!("Invalid related-message sort"),
                };
                self.related_ascending = if sort == self.related_sort {
                    !self.related_ascending
                } else {
                    sort != SortField::Date
                };
                self.related_sort = sort;
                self.related_before = None;
            },
            "related_mode" => {
                self.relationship = if self.relationship == Relationship::Thread {
                    Relationship::Correspondence
                } else {
                    Relationship::Thread
                };
                self.related_before = None;
            },
            _ => {},
        }
        Ok(())
    }
}
