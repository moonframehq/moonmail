use super::*;

impl Mail {
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
                    self.status = "Folder change queued for next Sync".into();
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
