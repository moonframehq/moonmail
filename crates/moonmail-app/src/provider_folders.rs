use super::*;

impl Mail {
    pub(super) fn confirm_expunge(&mut self, action: &str, message: &Data) -> Result<()> {
        let text = |key| {
            message
                .get(key)
                .and_then(Data::as_str)
                .context("Missing folder confirmation")
        };
        let number = |key| {
            message
                .get(key)
                .and_then(Data::as_i64)
                .context("Missing confirmation scope")
        };
        let account = text("account")?;
        let epoch: u32 = number("epoch")?.try_into()?;
        anyhow::ensure!(
            !self.demo
                && Account::load_for(&self.root, account)?.is_some_and(|a| (a.transport
                    == moonmail_network::Transport::Jmap)
                    == (epoch == 0))
                && message.get("confirmed").and_then(Data::as_bool) == Some(true),
            "Confirm deletion for a connected account"
        );
        let role = MailboxRole::parse(text("role")?).context("Missing folder role")?;
        let target = if action == "confirm_empty_folder" {
            moonmail_store::DeletionTarget::Through(number("upper")?.try_into()?)
        } else {
            moonmail_store::DeletionTarget::Message(
                MessageId(number("message")?.try_into()?),
                number("selected_uid")?.try_into()?,
            )
        };
        let count = self.archive_mut()?.queue_expunge(
            account,
            number("id")?,
            text("name")?,
            epoch,
            role,
            target,
        )?;
        self.workers.flush(account);
        self.status = format!("{count} permanent deletions queued for Sync");
        Ok(())
    }

    pub(super) fn handle_folder_edit(&mut self, action: &str, message: &Data) -> Result<()> {
        let text = |key| message.get(key).and_then(Data::as_str).unwrap_or_default();
        let id = || {
            message
                .get("id")
                .and_then(Data::as_i64)
                .context("Choose a folder operation")
        };
        match action {
            "folder_cancel" => self.archive()?.cancel_folder_edit(id()?)?,
            "folder_dismiss" => self.archive()?.dismiss_folder_edit(id()?)?,
            _ => {
                let account = text("account");
                let edit = match action {
                    "folder_create" => moonmail_store::FolderEdit::Create(text("name").into()),
                    "folder_rename" | "folder_delete" => {
                        let id = id()?;
                        let folder = self
                            .archive()?
                            .mailboxes()?
                            .into_iter()
                            .find(|folder| folder.id == id)
                            .context("Folder is no longer available")?;
                        anyhow::ensure!(
                            folder.account == account
                                && folder.name == text("source")
                                && message.get("epoch").and_then(Data::as_i64)
                                    == Some(i64::from(folder.epoch)),
                            "Folder changed; reopen its editor"
                        );
                        if action == "folder_delete" {
                            anyhow::ensure!(
                                text("confirm") == folder.name,
                                "Type the full folder name to delete it and its contents"
                            );
                            moonmail_store::FolderEdit::Delete(id)
                        } else {
                            moonmail_store::FolderEdit::Rename {
                                id,
                                name: text("name").into(),
                            }
                        }
                    },
                    _ => bail!("Unknown folder action"),
                };
                if self.demo || account == "local" {
                    self.archive_mut()?.edit_local_folder(account, edit)?;
                    self.status =
                        "Local folder updated; original message bytes remain archived".into();
                } else {
                    anyhow::ensure!(
                        Account::load_for(&self.root, account)?.is_some(),
                        "Connect this account before queuing a server folder change"
                    );
                    self.archive_mut()?.queue_folder_edit(account, edit)?;
                    self.workers.flush(account);
                    self.status = "Folder change queued".into();
                }
                if let Some(id) = self.query.mailbox
                    && !self
                        .archive()?
                        .mailboxes()?
                        .iter()
                        .any(|folder| folder.id == id)
                {
                    self.query.mailbox = None;
                }
                if let Some(account) = &self.query.account
                    && !self
                        .archive()?
                        .mailboxes()?
                        .iter()
                        .any(|folder| &folder.account == account)
                {
                    self.query.account = None;
                }
                self.query.before = None;
                self.expanded.clear();
                self.checked.clear();
                self.refresh()?;
            },
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tests::data, *};

    #[test]
    fn folder_actions_bind_names_epochs_accounts_and_deletion_confirmation() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut archive = Archive::open(dir.path())?;
        let remote = archive.mailbox("me@example.org", "Projects")?;
        archive.reconcile(remote, 7, &[])?;
        Account::fastmail("me@example.org")?.save(dir.path())?;
        drop(archive);
        let mut mail = Mail::new(dir.path().to_owned(), false);
        mail.initialize()?;
        mail.handle(data(
            json!({"action":"folder_create","account":"local","name":"Notes"}),
        ))?;
        let local = mail
            .archive()?
            .mailboxes()?
            .into_iter()
            .find(|folder| folder.account == "local")
            .context("local folder")?;
        let rename = json!({"action":"folder_rename","account":"local","id":local.id,"source":"Notes","epoch":0,"name":"Ideas"});
        mail.handle(data(rename.clone()))?;
        assert!(
            mail.handle(data(rename)).is_err(),
            "replayed old name cannot rename the new folder"
        );
        assert!(mail.handle(data(json!({"action":"folder_delete","account":"local","id":local.id,"source":"Ideas","epoch":0,"confirm":"Notes"}))).is_err());
        assert!(mail.handle(data(json!({"action":"folder_delete","account":"local","id":local.id,"source":"Ideas","epoch":1,"confirm":"Ideas"}))).is_err());
        mail.handle(data(json!({"action":"folder","id":local.id})))?;
        mail.handle(data(json!({"action":"folder_delete","account":"local","id":local.id,"source":"Ideas","epoch":0,"confirm":"Ideas"})))?;
        assert!(mail.query.mailbox.is_none());
        assert!(
            !mail
                .archive()?
                .mailboxes()?
                .iter()
                .any(|folder| folder.id == local.id)
        );
        assert!(mail.handle(data(json!({"action":"folder_rename","account":"other@example.org","id":remote,"source":"Projects","epoch":7,"name":"Work"}))).is_err());
        mail.handle(data(json!({"action":"folder_rename","account":"me@example.org","id":remote,"source":"Projects","epoch":7,"name":"Work"})))?;
        let pending = mail
            .archive()?
            .pending_folder_edit("me@example.org")?
            .context("queued remote edit")?;
        assert_eq!(mail.snapshot()?["unresolved_operations"], 1);
        assert!(
            mail.archive()?
                .mailboxes()?
                .iter()
                .any(|folder| folder.id == remote && folder.name == "Projects")
        );
        assert!(
            mail.snapshot()?["folder_operations"][0]
                .get("members")
                .is_none(),
            "UI history does not serialize frozen descendant inventories"
        );
        mail.handle(data(json!({"action":"folder_cancel","id":pending.id})))?;
        assert_eq!(mail.snapshot()?["unresolved_operations"], 0);
        assert!(
            mail.archive()?
                .pending_folder_edit("me@example.org")?
                .is_none()
        );
        Ok(())
    }
}

#[cfg(test)]
mod expunge_tests {
    use super::{super::tests::data, *};
    #[test]
    fn jmap_trash_confirmation_uses_the_bound_transport_and_current_folder() -> Result<()> {
        let dir = tempfile::tempdir()?;
        crate::fonts::Preferences {
            sync_paused: true,
            ..Default::default()
        }
        .save(dir.path())?;
        let mut account = Account::fastmail("test@example.org")?;
        account.transport = moonmail_network::Transport::Jmap;
        account.save(dir.path())?;
        let mut db = Archive::open(dir.path())?;
        db.bind_jmap(&account.email, "account")?;
        db.jmap_folders(
            &account.email,
            &[moonmail_store::JmapFolder {
                remote: "t".into(),
                path: "Deleted".into(),
                role: Some(MailboxRole::Trash),
            }],
            "f",
        )?;
        let mut mail = Mail::new(dir.path().to_owned(), false);
        mail.initialize()?;
        let mut request = mail.snapshot()?["deletion_folders"][0].clone();
        assert_eq!(request["epoch"], 0);
        assert_eq!(request["name"], "Deleted");
        request["action"] = json!("confirm_empty_folder");
        assert!(mail.handle(data(request.clone())).is_err());
        request["confirmed"] = json!(true);
        mail.handle(data(request.clone()))?;
        request["upper"] = json!(22);
        assert!(mail.handle(data(request.clone())).is_err());
        account.transport = moonmail_network::Transport::Imap;
        account.save(dir.path())?;
        assert!(mail.handle(data(request)).is_err());
        Ok(())
    }

    #[test]
    fn empty_folder_requires_explicit_confirmation_and_the_original_role_and_epoch() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let account = "me@example.org";
        Account::fastmail(account)?.save(dir.path())?;
        let mut archive = Archive::open(dir.path())?;
        let trash = archive.mailbox(account, "Deleted Items")?;
        archive.mailbox_server_role(trash, Some(MailboxRole::Trash))?;
        archive.reconcile(trash, 71, &[])?;
        let mut mail = Mail::new(dir.path().to_owned(), false);
        mail.initialize()?;
        let mut request = json!({"action":"confirm_empty_folder","id":trash,"account":account,"role":"trash","name":"Deleted Items","epoch":71,"upper":4});
        assert!(mail.handle(data(request.clone())).is_err());
        request["confirmed"] = json!(true);
        mail.handle(data(request.clone()))?;
        request["epoch"] = json!(72);
        assert!(mail.handle(data(request.clone())).is_err());
        request["epoch"] = json!(71);
        request["role"] = json!("drafts");
        assert!(mail.handle(data(request)).is_err());
        Ok(())
    }
}
