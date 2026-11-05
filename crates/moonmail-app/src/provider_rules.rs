use super::*;

impl Mail {
    pub(super) fn rule_snapshot(&self, snapshot: &mut serde_json::Value) -> Result<()> {
        let runs = self.archive()?.keyword_rule_runs(self.rule_before)?;
        snapshot["rule_next"] = json!(runs.last().filter(|_| runs.len() == 100).map(|r| r.id));
        snapshot["rule_before"] = json!(self.rule_before);
        snapshot["rule_runs"] = json!(runs);
        snapshot["keyword_rules"] = json!(self.archive()?.keyword_rules()?);
        Ok(())
    }

    pub(super) fn handle_rule(&mut self, action: &str, message: &Data) -> Result<()> {
        let text = |key| message.get(key).and_then(Data::as_str).unwrap_or_default();
        let integer = |key| {
            message
                .get(key)
                .and_then(Data::as_i64)
                .context("Missing rule identity")
        };
        match action {
            "rule_save" => {
                let mailbox = integer("mailbox")?;
                let folder = self
                    .archive()?
                    .mailboxes()?
                    .into_iter()
                    .find(|b| b.id == mailbox)
                    .context("Rule folder disappeared")?;
                anyhow::ensure!(
                    folder.account == text("account")
                        && folder.name == text("folder")
                        && i64::from(folder.epoch) == integer("epoch")?,
                    "Rule folder changed; reopen its editor"
                );
                let source = text("source");
                let predicate = if source.trim().is_empty() {
                    moonmail_core::MailPredicate::Account(folder.account)
                } else {
                    moonmail_search::parse_query(source)?
                };
                let rule = moonmail_store::KeywordRule {
                    id: integer("id")?,
                    revision: integer("revision")?,
                    mailbox,
                    name: text("name").into(),
                    source: source.into(),
                    predicate,
                    keyword: text("keyword").into(),
                    value: message
                        .get("value")
                        .and_then(Data::as_bool)
                        .context("Choose a rule action")?,
                    enabled: message
                        .get("enabled")
                        .and_then(Data::as_bool)
                        .context("Choose whether to enable this rule")?,
                };
                self.archive_mut()?.save_keyword_rule(&rule)?;
                self.status =
                    "Rule saved for future folder entries; use Edit to reopen this version".into();
            },
            "rule_delete" => self
                .archive()?
                .delete_keyword_rule(integer("id")?, integer("revision")?)?,
            "rule_apply" => {
                let ids = self.move_selection();
                self.validate_move_choice(message, &ids)?;
                let count = self.archive_mut()?.apply_keyword_rule(
                    integer("id")?,
                    integer("revision")?,
                    &ids,
                )?;
                self.refresh()?;
                self.status = format!(
                    "Rule applied to {count} matching selected message(s); inspect rule history for blocked actions"
                );
            },
            "rule_first" => self.rule_before = None,
            "rule_next" => {
                let id = integer("id")?;
                let rows = self.archive()?.keyword_rule_runs(self.rule_before)?;
                anyhow::ensure!(
                    rows.len() == 100 && rows.last().is_some_and(|r| r.id == id),
                    "Rule history page changed"
                );
                self.rule_before = Some(id);
            },
            _ => bail!("Unknown rule action"),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tests::data, *};

    #[test]
    fn rule_editor_validates_folder_revision_conditions_and_selected_manual_actions() -> Result<()>
    {
        let dir = tempfile::tempdir()?;
        let mut archive = Archive::open(dir.path())?;
        moonmail_store::seed_demo(&mut archive, &FileBlobs::new(dir.path()))?;
        drop(archive);
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        let ids = mail.move_selection();
        let folder = mail
            .archive()?
            .mailboxes()?
            .into_iter()
            .find(|b| b.name == "INBOX")
            .context("inbox")?;
        let request = json!({"action":"rule_save","id":0,"revision":0,"mailbox":folder.id,"account":folder.account,"folder":folder.name,"epoch":folder.epoch,"name":"Follow up","source":"","keyword":"followup","value":true,"enabled":true});
        let mut invalid = request.clone();
        invalid["source"] = json!("subject:Review");
        assert!(mail.handle(data(invalid)).is_err());
        let mut invalid = request.clone();
        invalid["account"] = json!("other@example.org");
        assert!(mail.handle(data(invalid)).is_err());
        mail.handle(data(request.clone()))?;
        assert!(mail.archive()?.message_keywords(ids[0])?.is_empty());
        assert!(
            mail.handle(data(request)).is_err(),
            "new-rule replay cannot create a duplicate name in the same folder"
        );
        let rule = mail.archive()?.keyword_rules()?.remove(0);
        let apply = json!({"action":"rule_apply","id":rule.id,"revision":rule.revision,"messages":ids,"scope":mail.query.mailbox});
        let mut invalid = apply.clone();
        invalid["messages"] = json!([99999]);
        assert!(mail.handle(data(invalid)).is_err());
        mail.handle(data(apply.clone()))?;
        assert_eq!(mail.snapshot()?["rule_runs"][0]["mode"], "manual");
        assert_eq!(mail.archive()?.message_keywords(ids[0])?, ["followup"]);
        let mut revised = rule.clone();
        revised.enabled = false;
        mail.archive_mut()?.save_keyword_rule(&revised)?;
        assert!(mail.handle(data(apply)).is_err());
        assert!(
            mail.handle(data(
                json!({"action":"rule_delete","id":rule.id,"revision":rule.revision})
            ))
            .is_err()
        );
        Ok(())
    }
}
