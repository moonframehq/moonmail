use super::*;

impl Mail {
    /// Prepare actual provider state synchronously before the native scene timer.
    /// The harness owns this synthetic archive and the exported reader files.
    pub(crate) fn capture_snapshot(
        root: &std::path::Path,
        view: &str,
    ) -> Result<serde_json::Value> {
        anyhow::ensure!(
            std::env::var_os("MOONMAIL_DEMO").is_some(),
            "Capture requires synthetic demo mode"
        );
        let mut mail = Self::new(root.to_owned(), true);
        mail.initialize()?;
        if view == "accounts" {
            for (provider, email) in [
                (moonmail_network::Provider::Google, "you@gmail.com"),
                (moonmail_network::Provider::Fastmail, "work@example.org"),
                (moonmail_network::Provider::Microsoft, "studio@outlook.com"),
            ] {
                Account::preset(provider, email)?.save(root)?;
            }
        }
        if view == "from-menu" {
            for email in ["you@example.com", "work@example.org", "studio@example.net"] {
                Account::fastmail(email)?.save(root)?;
            }
            mail.archive_mut()?
                .save_identity(&moonmail_store::Identity {
                    id: "studio-alias".into(),
                    account: "studio@example.net".into(),
                    name: "Design team".into(),
                    address: "design@example.net".into(),
                    ..Default::default()
                })?;
        }
        if matches!(view, "searches" | "search-results") {
            while mail.index_pending()? {}
            for (name, source) in [
                ("Unread mail", "is:unread"),
                ("Clara history", "(from:clara OR to:clara) NOT in:Trash"),
            ] {
                mail.handle(Data::map([
                    ("action", Data::String("search".into())),
                    ("value", Data::String(source.into())),
                ]))?;
                mail.handle(Data::map([
                    ("action", Data::String("search_save".into())),
                    ("name", Data::String(name.into())),
                ]))?;
            }
            let parent = mail
                .archive()?
                .saved_searches()?
                .into_iter()
                .find(|search| search.name == "Clara history")
                .context("Missing source search")?
                .id;
            mail.handle(Data::map([
                ("action", Data::String("search".into())),
                ("value", Data::String("is:unread".into())),
            ]))?;
            mail.handle(Data::map([
                ("action", Data::String("search_save".into())),
                ("name", Data::String("Unread Clara".into())),
                ("sources", Data::List(vec![Data::Integer(parent)])),
            ]))?;
            mail.handle(Data::map([
                ("action", Data::String("search_open".into())),
                ("id", Data::Integer(parent)),
            ]))?;
            if let Some(row) = mail.page.rows.first() {
                mail.select(row.id, true)?;
            }
        }
        if view == "outbox" {
            mail.archive_mut()?.reconcile_sent("you@example.com")?;
        }
        if view == "unread-navigation" {
            mail.selected = None;
            mail.seek_message(false, true)?;
        }
        let selected = mail
            .selected
            .as_ref()
            .context("Capture needs a selected message")?
            .summary
            .id;
        let action = match view {
            "identity-compose" => Some("compose"),
            "correspondence" => Some("related_next"),
            "forward" => Some("forward"),
            "attachments" => Some("attachments"),
            "images" => Some("reader_images"),
            "threads" | "columns" => Some("expand"),
            _ => None,
        };
        if view == "tags" {
            let account = mail.archive()?.message_account(selected)?;
            mail.archive_mut()?
                .set_keywords(&[selected], "project-work", true)?;
            mail.archive()?
                .set_keyword_label(&account, "project-work", "Work projects")?;
            mail.archive()?
                .set_keyword_label(&account, "$label1", "Follow up")?;
        }
        if view == "rules" {
            let folder = mail
                .archive()?
                .mailboxes()?
                .into_iter()
                .find(|b| b.name == "INBOX")
                .context("Rule source folder")?;
            let rule = moonmail_store::KeywordRule {
                id: 0,
                revision: 0,
                mailbox: folder.id,
                name: "Follow up on new mail".into(),
                source: "NOT keyword:finished".into(),
                predicate: moonmail_core::MailPredicate::Not(Box::new(
                    moonmail_core::MailPredicate::Keyword("finished".into()),
                )),
                keyword: "followup".into(),
                value: true,
                enabled: true,
            };
            let id = mail.archive_mut()?.save_keyword_rule(&rule)?;
            mail.archive_mut()?.apply_keyword_rule(id, 1, &[selected])?;
        }
        if view == "partitions" {
            let predicate = moonmail_core::MailPredicate::Account("you@example.com".into());
            let id = mail.archive_mut()?.save_search(
                "All demo senders",
                "account:you@example.com",
                &predicate,
            )?;
            mail.archive()?
                .set_search_partition(id, Some(moonmail_store::PartitionField::Sender))?;
            mail.handle(Data::map([
                ("action", Data::String("partition_open".into())),
                ("id", Data::Integer(id)),
            ]))?;
        }
        if let Some(action) = action {
            mail.handle(Data::map([
                ("action", Data::String(action.into())),
                ("message", Data::Integer(selected.0 as i64)),
                ("id", Data::Integer(selected.0 as i64)),
                ("anchor", Data::Integer(selected.0 as i64)),
            ]))?;
        }
        let mut snapshot = mail.snapshot()?;
        for key in ["html_url", "plain_url"] {
            if let Some(source) = snapshot[key].as_str().filter(|url| !url.is_empty()) {
                let mut url = url::Url::parse(source)?;
                url.set_query(None);
                let path = url
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("Invalid reader URL"))?;
                let destination = root.join(format!("capture-{key}.html"));
                std::fs::copy(path, &destination)?;
                snapshot[key] = json!(
                    url::Url::from_file_path(destination)
                        .map_err(|_| anyhow::anyhow!("Invalid capture path"))?
                        .to_string()
                );
            }
        }
        Ok(snapshot)
    }
}
