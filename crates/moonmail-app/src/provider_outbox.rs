use super::*;

impl Mail {
    pub(super) fn handle_outbox(&mut self, action: &str, message: &Data) -> Result<()> {
        if action == "outbox_first" {
            self.outbox_before = None;
            return Ok(());
        }
        if action == "outbox_next" {
            if let Some(next) = self.archive()?.outbox_page(self.outbox_before, 50)?.next {
                self.outbox_before = Some(next);
            }
            return Ok(());
        }
        let id = message
            .get("id")
            .and_then(Data::as_i64)
            .context("Missing outbox ID")?;
        match action {
            "outbox_sent" => {
                let item = self.archive()?.outbox_item(id)?;
                self.archive_mut()?.reconcile_sent(&item.account)?;
                let message = self.archive()?.outbox_item(id)?.sent_message.context("No unique matching copy in the synced Sent folder; Sync and check the folder role")?;
                self.select(message, true)?;
            },
            "outbox_cancel" => {
                self.archive()?.cancel_outbox(id)?;
                self.status = "Outbox message cancelled before submission".into();
            },
            "outbox_edit" => {
                let draft = self
                    .archive_mut()?
                    .restore_outbox_draft(id, &uuid::Uuid::new_v4().to_string())?;
                self.draft = Some(draft);
                self.draft_before = None;
                self.status = "Returned to drafts; original outbox entry cancelled".into();
            },
            "outbox_retry" => {
                anyhow::ensure!(!self.demo, "Sending is disabled in the synthetic demo");
                anyhow::ensure!(
                    self.job.is_none(),
                    "Wait for the current network operation to finish"
                );
                let account = self.archive()?.outbox_item(id)?.account;
                Account::load_for(&self.root, &account)?
                    .context("Connect the message's original account before retrying")?;
                self.archive()?.retry_outbox(id)?;
                self.launch(move |root, _| Ok(moonmail_network::send_queued(&root, id)?))?;
            },
            _ => {},
        }
        Ok(())
    }
}
