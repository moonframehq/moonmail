use super::*;

impl Mail {
    // Open only after a successful durable action and its published snapshot.
    // Deleting a different draft in the mailbox must not close the composer.
    pub(super) fn composer_window_event(&self, message: &Data) -> Option<&'static str> {
        match message.get("action").and_then(Data::as_str)? {
            "compose" | "reply" | "reply_all" | "forward" | "forward_text" | "draft_open"
            | "outbox_edit" => Some("open"),
            "draft_close" | "queue" | "queue_only" => Some("close"),
            "draft_discard"
                if self.draft.as_ref().is_some_and(|draft| {
                    message.get("id").and_then(Data::as_str) == Some(draft.id.as_str())
                }) =>
            {
                Some("close")
            },
            _ => None,
        }
    }

    pub(super) fn handle_draft(&mut self, action: &str, message: &Data) -> Result<()> {
        let text = |key| {
            message
                .get(key)
                .and_then(Data::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        match action {
            "compose" | "reply" | "reply_all" | "forward" | "forward_text" => {
                let configured = self.configured_account()?;
                let mut draft = if action == "compose" {
                    Draft {
                        identity: moonmail_network::choose_identity(
                            &self.archive()?.identities(&configured)?,
                            &configured,
                            None,
                        ),
                        account: configured,
                        ..Default::default()
                    }
                } else {
                    let detail = self
                        .selected
                        .as_ref()
                        .context("Select a message to reply to")?;
                    let headers = self.archive()?.reply_headers(
                        detail.summary.id,
                        &moonmail_mime::StalwartParser::default(),
                        &FileBlobs::new(&self.root),
                    )?;
                    let original = self.archive()?.message_account(detail.summary.id)?;
                    let account = if moonmail_network::validate_address(&original).is_ok() {
                        original
                    } else {
                        configured
                    };
                    let identities = self.archive()?.identities(&account)?;
                    let identity =
                        moonmail_network::choose_identity(&identities, &account, Some(&headers));
                    let mut draft = if matches!(action, "forward" | "forward_text") {
                        moonmail_network::compose_forward(detail, &headers, &account)?
                    } else {
                        moonmail_network::compose_reply_with_identities(
                            detail,
                            headers,
                            &account,
                            if action == "reply_all" {
                                moonmail_network::ReplyMode::All
                            } else {
                                moonmail_network::ReplyMode::Sender
                            },
                            &identities,
                        )?
                    };
                    draft.identity = identity;
                    draft
                };
                moonmail_network::insert_signature(&mut draft);
                if action == "forward" {
                    let id = self
                        .selected
                        .as_ref()
                        .context("Select a message to forward")?
                        .summary
                        .id;
                    self.copy_forward_attachments(&mut draft, id)?;
                }
                draft.id = uuid::Uuid::new_v4().to_string();
                self.archive()?.save_draft(&draft)?;
                self.draft = Some(draft);
                self.draft_before = None;
            },
            "draft_open" => {
                let draft = self
                    .archive()?
                    .draft(&text("id"))?
                    .context("Draft no longer exists")?;
                self.draft = Some(draft);
            },
            "draft_close" => {
                let draft = self.draft.as_ref().context("No open draft")?;
                anyhow::ensure!(text("id") == draft.id, "This compose view is stale");
                // Inputs are persisted before this ordered close request. Keep
                // the draft available; closing its window is not discarding it.
                self.archive()?.save_draft(draft)?;
            },
            "draft_account" => {
                let mut draft = self.draft.clone().context("Open a draft first")?;
                anyhow::ensure!(text("id") == draft.id, "This compose view is stale");
                let account = text("account");
                anyhow::ensure!(
                    self.sending_accounts()?
                        .iter()
                        .any(|item| item.email == account),
                    "This sending account is no longer connected"
                );
                let identity = if text("identity").is_empty() {
                    None
                } else {
                    Some(self.archive()?.identity(&account, &text("identity"))?)
                };
                // The SMTP account and frozen From/Reply-To identity change
                // together. Edited text, signatures, recipients and files stay.
                draft.account = account;
                draft.identity = identity;
                self.archive()?.save_draft(&draft)?;
                self.draft = Some(draft);
                self.status = "Sending account saved".into();
            },
            "draft_identity" | "draft_signature" => {
                let mut draft = self.draft.clone().context("Open a draft first")?;
                anyhow::ensure!(text("id") == draft.id, "This compose view is stale");
                if action == "draft_signature" {
                    moonmail_network::insert_signature(&mut draft);
                } else {
                    draft.identity = if text("identity").is_empty() {
                        None
                    } else {
                        Some(
                            self.archive()?
                                .identity(&draft.account, &text("identity"))?,
                        )
                    };
                }
                self.archive()?.save_draft(&draft)?;
                self.draft = Some(draft);
                self.status =
                    "Sender saved. Signature text changes only when explicitly inserted.".into();
            },
            "draft_discard" => {
                let id = text("id");
                self.archive()?.discard_draft(&id)?;
                if self.draft.as_ref().is_some_and(|draft| draft.id == id) {
                    self.draft = None;
                }
                self.draft_before = None;
            },
            "draft" => {
                // Late input events from an old compose view cannot overwrite
                // another draft, resurrect a deleted draft, or alter queued MIME.
                if let Some(mut draft) = self.draft.clone() {
                    if text("id") != draft.id {
                        return Ok(());
                    }
                    match text("field").as_str() {
                        "recipients" => draft.recipients = text("value"),
                        "cc" => draft.cc = text("value"),
                        "bcc" => draft.bcc = text("value"),
                        "subject" => draft.subject = text("value"),
                        "body" => draft.body = text("value"),
                        _ => return Ok(()),
                    }
                    self.archive()?.save_draft(&draft)?;
                    self.draft = Some(draft);
                }
            },
            "drafts_first" => self.draft_before = None,
            "drafts_next" => {
                if let Some(next) = self.archive()?.drafts(self.draft_before.as_ref(), 50)?.next {
                    self.draft_before = Some(next);
                }
            },
            _ => {},
        }
        Ok(())
    }
}
