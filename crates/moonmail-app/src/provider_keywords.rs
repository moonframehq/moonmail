use super::*;

impl Mail {
    fn tag_account(&self) -> Result<String> {
        Ok(self
            .keyword_account
            .clone()
            .or(self.move_account(&self.move_selection())?)
            .or(Account::load(&self.root)?.map(|a| a.email))
            .unwrap_or_else(|| "local".into()))
    }

    pub(super) fn keyword_snapshot(&self, snapshot: &mut serde_json::Value) -> Result<()> {
        let account = self.tag_account()?;
        let operations = self
            .archive()?
            .keyword_operations(&account, self.keyword_after)?;
        snapshot["keyword_next"] = json!(
            operations
                .last()
                .filter(|_| operations.len() == 100)
                .map(|op| op.id)
        );
        snapshot["keyword_after"] = json!(self.keyword_after);
        snapshot["keyword_operations"] = json!(operations);
        snapshot["keyword_labels"] = json!(self.archive()?.keyword_labels(&account)?);
        snapshot["keyword_account"] = json!(account);
        snapshot["selected_keywords"] = json!(
            self.selected
                .as_ref()
                .map(|m| self
                    .archive()?
                    .message_keywords(m.summary.id)
                    .map_err(anyhow::Error::from))
                .transpose()?
                .unwrap_or_default()
        );
        Ok(())
    }

    pub(super) fn handle_keyword(&mut self, action: &str, message: &Data) -> Result<()> {
        let text = |key| message.get(key).and_then(Data::as_str).unwrap_or_default();
        let account = text("account");
        let valid = self
            .archive()?
            .mailboxes()?
            .iter()
            .any(|b| b.account == account)
            || Account::load_for(&self.root, account)?.is_some()
            || account == "local";
        anyhow::ensure!(valid, "Choose an existing account");
        if action == "keyword_account" {
            self.keyword_account = Some(account.into());
            self.keyword_after = 0;
            return Ok(());
        }
        anyhow::ensure!(
            self.tag_account()? == account,
            "Tag account changed; reopen Tags"
        );
        match action {
            "keyword_set" => {
                let ids = self.move_selection();
                self.validate_move_choice(message, &ids)?;
                anyhow::ensure!(
                    self.move_account(&ids)?.as_deref() == Some(account),
                    "Select messages from this tag account"
                );
                let value = message
                    .get("value")
                    .and_then(Data::as_bool)
                    .context("Choose add or remove")?;
                let count = self
                    .archive_mut()?
                    .set_keywords(&ids, text("keyword"), value)?;
                self.refresh()?;
                self.status = format!(
                    "Updated keywords on {count} selected message(s); remote edits upload on Sync"
                );
            },
            "keyword_label" => {
                self.archive()?
                    .set_keyword_label(account, text("keyword"), text("label"))?;
                self.refresh()?;
                self.status = "Tag label saved for this account".into();
            },
            "keyword_first" => self.keyword_after = 0,
            "keyword_next" => {
                let id = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .context("Missing keyword page")?;
                let page = self
                    .archive()?
                    .keyword_operations(account, self.keyword_after)?;
                anyhow::ensure!(
                    page.len() == 100 && page.last().is_some_and(|op| op.id == id),
                    "Keyword page changed"
                );
                self.keyword_after = id;
            },
            "keyword_retry" | "keyword_discard" => {
                anyhow::ensure!(
                    self.job.is_none(),
                    "Wait for Sync to finish before resolving keyword operations"
                );
                let id = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .filter(|id| *id > 0)
                    .context("Choose a keyword operation")?;
                anyhow::ensure!(
                    self.archive()?
                        .keyword_operations(account, id - 1)?
                        .first()
                        .is_some_and(|op| op.id == id),
                    "Keyword operation changed or belongs to another account"
                );
                if action == "keyword_retry" {
                    self.archive()?.retry_keyword(id)?;
                } else {
                    self.archive()?.discard_keyword(id)?;
                }
                self.status = "Keyword recovery saved; Sync to refresh server state".into();
            },
            _ => bail!("Unknown keyword action"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tests::data, *};

    #[test]
    fn tag_actions_bind_selection_scope_account_and_recovery_operation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut archive = Archive::open(dir.path())?;
        moonmail_store::seed_demo(&mut archive, &FileBlobs::new(dir.path()))?;
        archive.mailbox("other@example.org", "INBOX")?;
        drop(archive);
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        let ids = mail.move_selection();
        let account = mail.move_account(&ids)?.context("selection account")?;
        let action = json!({"action":"keyword_set","account":account,"messages":ids,"scope":mail.query.mailbox,"keyword":"Work","value":true});
        mail.handle(data(action.clone()))?;
        assert_eq!(mail.snapshot()?["selected_keywords"], json!(["work"]));
        let mut stale = action.clone();
        stale["messages"] = json!([99999]);
        assert!(mail.handle(data(stale)).is_err());
        let mut stale = action.clone();
        stale["scope"] = json!(99999);
        assert!(mail.handle(data(stale)).is_err());
        let mut stale = action.clone();
        stale["account"] = json!("other@example.org");
        assert!(mail.handle(data(stale)).is_err());
        mail.handle(data(
            json!({"action":"keyword_account","account":"other@example.org"}),
        ))?;
        assert!(mail.handle(data(action)).is_err());
        mail.handle(data(json!({"action":"keyword_label","account":"other@example.org","keyword":"Work","label":"Other account"})))?;
        assert!(mail.archive()?.keyword_labels(&account)?.is_empty());
        assert_eq!(
            mail.snapshot()?["keyword_labels"][0]["label"],
            "Other account"
        );
        let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
        db.execute(
            "UPDATE occurrences SET epoch=7,uid=1 WHERE message_id=?1",
            [ids[0].0 as i64],
        )?;
        mail.archive_mut()?.set_keywords(&ids, "pending", true)?;
        let op = mail.archive()?.keyword_operations(&account, 0)?[0].id;
        mail.archive()?.block_keyword(op, "Unsupported")?;
        assert!(
            mail.handle(data(
                json!({"action":"keyword_discard","account":"other@example.org","id":op})
            ))
            .is_err()
        );
        mail.handle(data(json!({"action":"keyword_account","account":account})))?;
        assert_eq!(mail.snapshot()?["unresolved_operations"], 1);
        mail.handle(data(
            json!({"action":"keyword_retry","account":account,"id":op}),
        ))?;
        assert_eq!(
            mail.archive()?.keyword_operations(&account, 0)?[0].state,
            "queued"
        );
        mail.archive()?.block_keyword(op, "Unsupported")?;
        mail.handle(data(
            json!({"action":"keyword_discard","account":account,"id":op}),
        ))?;
        assert_eq!(mail.snapshot()?["unresolved_operations"], 0);
        Ok(())
    }
}
