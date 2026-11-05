use super::*;

impl Mail {
    /// Seek with the repository's keyset sort, preserving folder/search/flag
    /// scope. Unread filtering happens after grouping, so group cursors and
    /// representatives cannot change merely because an unread child is hidden.
    pub(super) fn seek_message(&mut self, up: bool, unread: bool) -> Result<()> {
        let mut scope = self.query.clone();
        scope.before = None;
        scope.threaded |= !unread;
        let mut anchor = scope.clone();
        if let Some(selected) = &self.selected {
            if scope.threaded {
                anchor.thread = Some(selected.summary.id);
            } else {
                anchor.ids = Some(vec![selected.summary.id]);
            }
        }
        let position = if self.selected.is_some() {
            self.archive()?.page(&anchor, 1)?.first
        } else {
            None
        };
        let mut seek = scope.clone();
        seek.ascending ^= up;
        seek.unread_only = unread;
        seek.before = position;
        let target = self.archive()?.page(&seek, 1)?;
        let Some(row) = target.rows.first() else {
            self.status = if unread {
                "No more unread messages in this direction"
            } else {
                "No more threads in this direction"
            }
            .into();
            return Ok(());
        };
        let group = row.id;
        let mut preceding = scope.clone();
        preceding.ascending = !scope.ascending;
        preceding.before = target.first;
        let before = self.archive()?.page(&preceding, 1)?.last;
        let mut selected = group;
        let mut expanded_before = None;
        if unread && scope.threaded {
            let mut children = scope.clone();
            children.threaded = false;
            children.thread = Some(group);
            children.sort = SortField::Date;
            children.ascending = true;
            children.unread_only = true;
            let first = self.archive()?.page(&children, 1)?;
            if let Some(child) = first.rows.first() {
                selected = child.id;
                children.unread_only = false;
                children.ascending = false;
                children.before = first.first;
                expanded_before = self.archive()?.page(&children, 1)?.last;
            }
        }
        self.query.threaded = scope.threaded;
        self.query.before = before;
        self.checked.clear();
        self.expanded.clear();
        self.refresh()?;
        if selected != group {
            self.expanded.insert(group, expanded_before);
        }
        self.select(selected, true)?;
        self.status = if unread {
            "Unread message selected"
        } else {
            "Thread selected"
        }
        .into();
        Ok(())
    }

    pub(super) fn next_page(&mut self) -> Result<bool> {
        let Some(next) = self.page.next.clone() else {
            return Ok(false);
        };
        self.query.before = Some(next);
        self.expanded.clear();
        self.checked.clear();
        self.refresh()?;
        Ok(true)
    }

    pub(super) fn previous_page(&mut self) -> Result<bool> {
        if self.query.before.is_none() {
            return Ok(false);
        }
        let mut reverse = self.query.clone();
        reverse.ascending = !reverse.ascending;
        reverse.before = self.page.first.clone().or(self.query.before.clone());
        // One extra row supplies the exclusive start of the preceding 75 rows.
        // Bounds retain group activity and collation keys even on the final page.
        let previous = self.archive()?.page(&reverse, 76)?;
        if previous.rows.is_empty() {
            return Ok(false);
        }
        self.query.before = if previous.rows.len() == 76 {
            previous.last
        } else {
            None
        };
        self.expanded.clear();
        self.checked.clear();
        self.refresh()?;
        Ok(true)
    }

    pub(super) fn navigate_message(&mut self, up: bool) -> Result<()> {
        let visible: Vec<_> = self
            .listing()?
            .into_iter()
            .filter(|row| row["thread_pager"] != true)
            .filter_map(|row| row["id"].as_u64().map(MessageId))
            .collect();
        let current = self
            .selected
            .as_ref()
            .and_then(|detail| visible.iter().position(|id| *id == detail.summary.id));
        let next = match current {
            Some(index) if up && index > 0 => visible.get(index - 1).copied(),
            Some(index) if !up && index + 1 < visible.len() => visible.get(index + 1).copied(),
            Some(_) if up && self.previous_page()? => self.page.rows.last().map(|row| row.id),
            Some(_) if !up && self.next_page()? => self.page.rows.first().map(|row| row.id),
            None => if up { visible.last() } else { visible.first() }.copied(),
            _ => None,
        };
        if let Some(id) = next {
            self.select(id, true)?;
        }
        Ok(())
    }

    pub(super) fn move_selection(&self) -> Vec<MessageId> {
        let mut ids: Vec<_> = self.checked.iter().copied().collect();
        if ids.is_empty()
            && let Some(detail) = &self.selected
        {
            ids.push(detail.summary.id);
        }
        ids.sort_unstable();
        ids
    }

    pub(super) fn move_account(&self, ids: &[MessageId]) -> Result<Option<String>> {
        let mut account = None;
        for id in ids {
            let next = self.archive()?.message_account(*id)?;
            if account.as_ref().is_some_and(|account| account != &next) {
                return Ok(None);
            }
            account = Some(next);
        }
        Ok(account)
    }

    pub(super) fn validate_move_choice(&self, message: &Data, ids: &[MessageId]) -> Result<()> {
        let Some(Data::List(expected)) = message.get("messages") else {
            bail!("Reopen the selection action");
        };
        let expected = expected
            .iter()
            .map(|id| {
                id.as_i64()
                    .filter(|id| *id > 0)
                    .map(|id| MessageId(id as u64))
                    .context("Invalid message selection")
            })
            .collect::<Result<Vec<_>>>()?;
        anyhow::ensure!(
            expected == ids
                && !ids.is_empty()
                && message.get("scope").and_then(Data::as_i64) == self.query.mailbox,
            "Selection changed; reopen the selection action"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tests::data, *};

    #[test]
    fn transfer_choosers_reject_stale_selection_scope_and_cross_account_targets() -> Result<()> {
        for action in ["choose_move", "choose_copy"] {
            let dir = tempfile::tempdir()?;
            let mut archive = Archive::open(dir.path())?;
            let inbox = archive.mailbox("you@example.com", "INBOX")?;
            let destination = archive.mailbox("you@example.com", "Projects/Review")?;
            let other = archive.mailbox("other@example.com", "INBOX")?;
            let mut ids = Vec::new();
            for n in 0..3 {
                ids.push(archive.ingest(
                    &moonmail_mime::StalwartParser::default(),
                    &FileBlobs::new(dir.path()),
                    moonmail_store::IncomingMessage {
                        account: if n == 2 {
                            "other@example.com"
                        } else {
                            "you@example.com"
                        },
                        mailbox: if n == 2 { other } else { inbox },
                        origin: &n.to_string(),
                        epoch: 0,
                        uid: None,
                        seen: true,
                        starred: false,
                        received_at: n,
                        raw: b"From: peer@example.org\r\nSubject: Chooser\r\n\r\nSynthetic",
                    },
                )?);
            }
            drop(archive);
            let mut mail = Mail::new(dir.path().to_owned(), true);
            mail.initialize()?;
            mail.handle(data(json!({"action":"folder","id":inbox})))?;
            mail.select(ids[0], true)?;
            let request =
                json!({"action":action,"id":destination,"messages":[ids[0]],"scope":inbox});
            mail.select(ids[1], true)?;
            assert!(mail.handle(data(request.clone())).is_err());
            mail.select(ids[0], true)?;
            mail.query.mailbox = None;
            assert!(mail.handle(data(request.clone())).is_err());
            mail.query.mailbox = Some(inbox);
            let flag = json!({"action":"selection_flag","messages":[ids[0]],"scope":inbox,"kind":"seen","value":false});
            mail.select(ids[1], true)?;
            assert!(mail.handle(data(flag.clone())).is_err());
            mail.checked.extend([ids[0], ids[1]]);
            mail.handle(data(json!({"action":"selection_flag","messages":[ids[0],ids[1]],"scope":inbox,"kind":"seen","value":false})))?;
            assert!(mail.archive()?.detail(ids[0])?.summary.unread);
            assert!(mail.archive()?.detail(ids[1])?.summary.unread);
            assert!(
                mail.selected
                    .as_ref()
                    .is_some_and(|detail| detail.summary.unread),
                "displayed message stays unread after bulk action"
            );
            mail.checked.clear();
            mail.select(ids[0], true)?;
            assert!(
                mail.handle(data(
                    json!({"action":action,"id":other,"messages":[ids[0]],"scope":inbox})
                ))
                .is_err()
            );
            mail.handle(data(request.clone()))?;
            if action == "choose_move" {
                assert!(
                    mail.handle(data(request)).is_err(),
                    "a repeated local move cannot reapply its old source"
                );
            } else {
                mail.handle(data(request))?;
                assert_eq!(
                    mail.archive()?
                        .page(
                            &MailQuery {
                                mailbox: Some(inbox),
                                ..Default::default()
                            },
                            10
                        )?
                        .rows
                        .len(),
                    2
                );
            }
            let moved = mail.archive()?.page(
                &MailQuery {
                    mailbox: Some(destination),
                    ..Default::default()
                },
                10,
            )?;
            assert_eq!(moved.rows.len(), 1);
            assert_eq!(moved.rows[0].id, ids[0]);
            mail.checked.extend([ids[1], ids[2]]);
            assert!(mail.move_account(&mail.move_selection())?.is_none());
            assert!(mail.handle(data(json!({"action":action,"id":destination,"messages":[ids[1],ids[2]],"scope":inbox}))).is_err());
            assert!(
                mail.handle(data(json!({"action":"folder","id":9999})))
                    .is_err()
            );
            assert_eq!(mail.query.mailbox, Some(inbox));
        }
        Ok(())
    }
}
