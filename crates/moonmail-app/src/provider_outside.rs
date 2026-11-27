//! Changes committed by another archive client while the app runs: the agent
//! CLI, the MCP server, or a second tool. SQLite's `data_version` moves on
//! each outside commit; the activity log records every agent tool call, and
//! agent requests ask the app to show something, report what its window
//! shows, or do what a click does. Agents ask the user before a change with
//! their own permission prompt; sending is also held here, whatever the
//! agent does: a `send` request waits in the assistant panel until the user
//! presses Send, which approves the digest of exactly what they were shown.

use super::*;

/// UI actions that send mail or answer a send request: only the user's own
/// click in the window performs them, never an agent's `app_action`.
pub(crate) const SENDING: &[&str] = &["agent_request", "queue", "send", "outbox_retry"];

/// The newest agent calls shown in the window.
const RECENT: usize = 50;
/// Rows of the list `app_state` reports.
const STATE_ROWS: usize = 50;

#[derive(Default)]
pub(super) struct Outside {
    version: i64,
    seen: i64,
    /// Folder the chat panel's agent works in, when it could be prepared.
    pub(super) agent_dir: Option<PathBuf>,
    pub(super) recent: Vec<moonmail_store::Activity>,
    /// What the composer window should do on the next tick: `open` or `close`.
    pub(super) composer: Option<&'static str>,
    /// Each pending send's one-time key, kept only in this window's memory
    /// and given only to its card: an agent can compute a draft's digest,
    /// but not this, so it cannot answer its own request.
    keys: std::cell::RefCell<HashMap<i64, String>>,
}

impl Mail {
    pub(super) fn watch_outside(&mut self) -> Result<()> {
        let archive = self.archive()?;
        let version = archive.data_version()?;
        let seen = archive.latest_activity()?;
        let recent = archive.activity_after(seen.saturating_sub(RECENT as i64), RECENT)?;
        // Requests to show something were for a window that has since
        // closed; sends still wait for the user.
        for request in archive.pending_requests()? {
            if request.kind != "send" {
                archive.settle_request(request.id, "expired", "Moonmail was not open")?;
            }
        }
        self.outside.version = version;
        self.outside.seen = seen;
        self.outside.recent = recent;
        match crate::agent::workspace::prepare(&self.root, self.demo) {
            Ok(dir) => self.outside.agent_dir = Some(dir),
            Err(error) => tracing::warn!("agent workspace unavailable: {error:#}"),
        }
        Ok(())
    }

    /// True when another client committed since the last call. A new agent
    /// write also picks up added accounts and wakes workers to upload.
    pub(super) fn outside_changes(&mut self) -> Result<bool> {
        let Some(archive) = &self.archive else {
            return Ok(false);
        };
        let version = archive.data_version()?;
        if version == self.outside.version {
            return Ok(false);
        }
        self.outside.version = version;
        self.follow_open_draft()?;
        self.serve_requests()?;
        let entries = self.archive()?.activity_after(self.outside.seen, 200)?;
        let Some(last) = entries.last() else {
            return Ok(true);
        };
        self.outside.seen = last.id;
        let writes: Vec<_> = entries
            .iter()
            .filter(|entry| entry.writes && entry.ok)
            .collect();
        if let Some(newest) = writes.last() {
            self.status = match writes.len() {
                1 => format!("Agent: {}", newest.summary),
                count => format!("Agent: {} (and {} more)", newest.summary, count - 1),
            };
            self.configure_workers()?;
            if self.workers.running() {
                self.workers.flush_all();
            }
        }
        self.outside.recent.extend(entries);
        let excess = self.outside.recent.len().saturating_sub(RECENT);
        self.outside.recent.drain(..excess);
        Ok(true)
    }

    /// The composer shows what the agent wrote into its draft, and closes
    /// when the draft was discarded or sent.
    fn follow_open_draft(&mut self) -> Result<()> {
        if let Some(open) = &self.draft {
            self.draft = self.archive()?.draft(&open.id)?;
            if self.draft.is_none() {
                self.outside.composer = Some("close");
            }
        }
        Ok(())
    }

    /// Carry out requests, oldest first, answering each.
    fn serve_requests(&mut self) -> Result<()> {
        for request in self.archive()?.pending_requests()? {
            if request.kind == "send" {
                continue;
            }
            let (state, detail) = match self.serve(&request) {
                Ok(answer) => (
                    "done",
                    answer.map(|answer| answer.to_string()).unwrap_or_default(),
                ),
                Err(error) => ("failed", format!("{error:#}")),
            };
            self.archive()?.settle_request(request.id, state, &detail)?;
        }
        Ok(())
    }

    fn serve(
        &mut self,
        request: &moonmail_store::AgentRequest,
    ) -> Result<Option<serde_json::Value>> {
        match request.kind.as_str() {
            "open_draft" => {
                let draft = self
                    .archive()?
                    .draft(&request.target)?
                    .context("The draft no longer exists")?;
                self.draft = Some(draft);
                self.draft_before = None;
                self.outside.composer = Some("open");
            },
            "show_message" => {
                let id = request.target.parse().context("Invalid message ID")?;
                self.select(MessageId(id), true)?;
            },
            "show_search" => {
                self.set_search(request.target.clone(), None, true)?;
                if let Some(row) = self.page.rows.first() {
                    self.select(row.id, true)?;
                }
            },
            "sync" => self.start_sync()?,
            "state" => return Ok(Some(self.window_state()?)),
            "action" => {
                let message: serde_json::Value =
                    serde_json::from_str(&request.target).context("Invalid UI action")?;
                let action = message.get("action").and_then(serde_json::Value::as_str);
                anyhow::ensure!(action.is_some(), "Name the UI action");
                // Only the user's click sends or approves a send.
                anyhow::ensure!(
                    !action.is_some_and(|action| SENDING.contains(&action)),
                    "Only the user sends mail or approves a send; ask with send_draft"
                );
                let message = data_from_json(message);
                let composer = self.composer_window_event(&message);
                self.error.clear();
                self.handle(message)?;
                anyhow::ensure!(self.error.is_empty(), "{}", self.error);
                if composer.is_some() {
                    self.outside.composer = composer;
                }
                self.refresh()?;
                return Ok(Some(self.window_state()?));
            },
            _ => bail!("Moonmail does not know this request"),
        }
        Ok(None)
    }

    /// What the window shows, for `app_state` and after an action.
    fn window_state(&self) -> Result<serde_json::Value> {
        let archive = self.archive()?;
        let rows: Vec<_> = self
            .page
            .rows
            .iter()
            .take(STATE_ROWS)
            .map(|row| {
                json!({ "id": row.id, "subject": row.subject, "sender": row.sender, "date": row.date_label,
                    "unread": row.unread, "starred": row.starred, "checked": self.checked.contains(&row.id) })
            })
            .collect();
        let folder = match self.query.mailbox {
            Some(id) => archive
                .mailboxes()?
                .into_iter()
                .find(|folder| folder.id == id)
                .map(|folder| json!({ "id": folder.id, "name": folder.name, "account": folder.account })),
            None => None,
        };
        Ok(json!({
            "folder": folder,
            "account": self.query.account,
            "filter": match self.query.filter { ViewFilter::All => "all", ViewFilter::Unread => "unread", ViewFilter::Starred => "starred" },
            "search": self.search,
            "rows": rows,
            "more": self.page.next.is_some(),
            "selected": self.selected.as_ref().map(|detail| &detail.summary),
            "checked": self.checked.iter().collect::<Vec<_>>(),
            "draft": self.draft,
            "layout": self.preferences.layout,
            "busy": self.job.is_some(),
            "status": self.status,
            "error": self.error,
        }))
    }

    /// A send waiting for the user: the draft as it is now, and the digest
    /// their Send must name.
    fn approval(
        &self,
        request: &moonmail_store::AgentRequest,
    ) -> Result<Option<serde_json::Value>> {
        Ok(self.archive()?.draft(&request.target)?.map(|draft| {
            let body: String = draft.body.chars().take(1200).collect();
            json!({
                "id": request.id, "digest": digest(&draft), "draft": draft.id,
                "key": self.outside.keys.borrow_mut().entry(request.id)
                    .or_insert_with(|| uuid::Uuid::new_v4().to_string()).clone(),
                "subject": draft.subject, "from": draft.sender(), "to": draft.recipients,
                "cc": draft.cc, "bcc": draft.bcc, "attachments": draft.attachments.len(),
                "body": body, "truncated": body.len() < draft.body.len(),
            })
        }))
    }

    /// `agent_request`: the user's Send or Deny on a send request.
    pub(super) fn handle_agent_request(&mut self, message: &Data) -> Result<()> {
        let id = message
            .get("id")
            .and_then(Data::as_i64)
            .context("Missing request")?;
        let request = self
            .archive()?
            .agent_request(id)?
            .filter(|request| request.state == "pending" && request.kind == "send")
            .context("The assistant's request was already answered")?;
        if message.get("decision").and_then(Data::as_str) != Some("approve") {
            self.archive()?
                .settle_request(id, "denied", "The user declined")?;
            self.status = "Declined the assistant's send".into();
            return Ok(());
        }
        let key = message.get("key").and_then(Data::as_str);
        anyhow::ensure!(
            key.is_some() && self.outside.keys.borrow().get(&id).map(String::as_str) == key,
            "Only the Send button in the assistant panel approves a send"
        );
        self.outside.keys.borrow_mut().remove(&id);
        let shown = message.get("digest").and_then(Data::as_str);
        let result = self.approve(&request, shown);
        let (state, detail) = match &result {
            Ok(detail) => ("done", detail.clone()),
            Err(error) => ("failed", format!("{error:#}")),
        };
        self.archive()?.settle_request(id, state, &detail)?;
        result.map(|detail| self.status = detail)
    }

    fn approve(
        &mut self,
        request: &moonmail_store::AgentRequest,
        shown: Option<&str>,
    ) -> Result<String> {
        let current = self
            .approval(request)?
            .context("The draft no longer exists")?;
        anyhow::ensure!(
            shown.is_some() && current["digest"].as_str() == shown,
            "The draft changed after you saw it; review it again"
        );
        let draft = self
            .archive()?
            .draft(&request.target)?
            .context("The draft no longer exists")?;
        let open = self.draft.as_ref().is_some_and(|open| open.id == draft.id);
        let subject = draft.subject.clone();
        self.queue_draft(draft, true)?;
        if open {
            self.outside.composer = Some("close");
        }
        Ok(format!("Sending “{subject}”"))
    }

    pub(super) fn outside_snapshot(&self, snapshot: &mut serde_json::Value) -> Result<()> {
        let mut approvals = Vec::new();
        for request in self.archive()?.pending_requests()? {
            if request.kind == "send"
                && let Some(approval) = self.approval(&request)?
            {
                approvals.push(approval);
            }
        }
        snapshot["agent"] = json!({
            "dir": self.outside.agent_dir.as_ref().map(|dir| dir.display().to_string()),
            "activity": self.outside.recent,
            "approvals": approvals,
        });
        Ok(())
    }
}

/// What an approval binds to: the JSON form of the draft the user was shown.
fn digest(value: &impl serde::Serialize) -> String {
    moonmail_store::content_digest(serde_json::to_vec(value).unwrap_or_default().as_slice())
}

/// A UI message from JSON, as the window's Luau would send it.
pub(super) fn data_from_json(value: serde_json::Value) -> Data {
    match value {
        serde_json::Value::Null => Data::Nil,
        serde_json::Value::Bool(value) => Data::Bool(value),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(value) => Data::Integer(value),
            None => Data::Number(number.as_f64().unwrap_or_default()),
        },
        serde_json::Value::String(value) => Data::String(value),
        serde_json::Value::Array(values) => {
            Data::List(values.into_iter().map(data_from_json).collect())
        },
        serde_json::Value::Object(fields) => Data::map(
            fields
                .into_iter()
                .map(|(key, value)| (key, data_from_json(value))),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{super::tests::data, *};

    #[test]
    fn agent_writes_refresh_the_open_window_and_report_activity() -> Result<()> {
        let dir = tempfile::tempdir()?;
        moonmail_store::seed_demo(&mut Archive::open(dir.path())?, &FileBlobs::new(dir.path()))?;
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        mail.handle(data(json!({"action":"filter","value":"unread"})))?;
        assert!(!mail.outside_changes()?, "nothing changed yet");
        let first = mail.page.rows.first().context("unread row")?.id;

        let args = json!({"ids":[first.0],"flag":"seen","value":true});
        crate::agent::tools::call(
            dir.path(),
            "mcp",
            "set_flag",
            args.as_object().context("args")?,
        )?;
        assert!(mail.outside_changes()?);
        assert_eq!(mail.status, "Agent: Marked 1 message(s) read");
        mail.refresh()?;
        assert!(
            mail.page.rows.iter().all(|row| row.id != first),
            "the read message leaves Unread"
        );
        let snapshot = mail.snapshot()?;
        assert_eq!(snapshot["agent"]["activity"][0]["source"], "mcp");
        assert!(
            snapshot["agent"]["dir"]
                .as_str()
                .is_some_and(|dir| dir.ends_with("agent"))
        );
        assert!(!mail.outside_changes()?, "seen once");
        Ok(())
    }

    fn tool(
        root: &std::path::Path,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        crate::agent::tools::call(root, "mcp", name, args.as_object().context("args")?)
    }

    /// Runs a tool on another thread, as an agent's process would, while
    /// the window keeps ticking until it answers.
    fn answered(
        mail: &mut Mail,
        root: &std::path::Path,
        name: &str,
        args: serde_json::Value,
    ) -> Result<serde_json::Value> {
        std::thread::scope(|scope| {
            let call = scope.spawn(|| tool(root, name, args));
            while !call.is_finished() {
                mail.outside_changes()?;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            call.join().map_err(|_| anyhow::anyhow!("tool thread"))?
        })
    }

    #[test]
    fn agent_writes_mail_in_the_composer_like_the_user_and_everything_is_logged() -> Result<()> {
        let dir = tempfile::tempdir()?;
        moonmail_store::seed_demo(&mut Archive::open(dir.path())?, &FileBlobs::new(dir.path()))?;
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        let reply_to = mail.selected.as_ref().context("selection")?.summary.id.0;

        // A new draft opens in the composer, filled in.
        let made = tool(
            dir.path(),
            "create_draft",
            json!({"reply_to":reply_to,"body":"Thursday works.","account":"you@example.com"}),
        )?;
        let draft = made["id"].as_str().context("draft id")?.to_owned();
        assert!(mail.outside_changes()?);
        assert_eq!(mail.outside.composer.take(), Some("open"));
        assert!(
            mail.draft
                .as_ref()
                .is_some_and(|open| open.id == draft && open.body.starts_with("Thursday works."))
        );

        // The open composer follows the agent's edits, and closes when the draft goes.
        tool(
            dir.path(),
            "update_draft",
            json!({"id":draft,"append":"See you then."}),
        )?;
        assert!(mail.outside_changes()?);
        assert!(
            mail.draft
                .as_ref()
                .is_some_and(|open| open.body.contains("See you then."))
        );

        // UI actions do what a click does: typing in the open draft, then reading the window.
        let typed = answered(
            &mut mail,
            dir.path(),
            "app_action",
            json!({"action":"draft","args":{"id":draft,"field":"subject","value":"Thursday"}}),
        )?;
        assert_eq!(typed["state"], "done");
        assert_eq!(typed["app"]["draft"]["subject"], "Thursday");
        let state = answered(&mut mail, dir.path(), "app_state", json!({}))?;
        assert_eq!(state["app"]["draft"]["id"], draft.as_str());
        let failed = answered(
            &mut mail,
            dir.path(),
            "app_action",
            json!({"action":"thread_read","args":{"id":-1}}),
        );
        assert!(failed.is_err(), "a failing action reports its error");
        assert!(
            tool(
                dir.path(),
                "app_action",
                json!({"action":"send","args":{"id":1}})
            )
            .is_err(),
            "only the user sends from the Outbox"
        );
        tool(dir.path(), "delete_draft", json!({"id":draft}))?;
        assert!(mail.outside_changes()?);
        assert!(mail.draft.is_none());
        assert_eq!(mail.outside.composer.take(), Some("close"));

        // Every call is logged, reads and failures included; only writes reach the status bar.
        let log = mail.archive()?.activity_after(0, 100)?;
        let tools: Vec<_> = log.iter().map(|entry| entry.tool.as_str()).collect();
        for name in [
            "create_draft",
            "update_draft",
            "app_action",
            "app_state",
            "delete_draft",
        ] {
            assert!(tools.contains(&name), "{name} logged");
        }
        assert!(log.iter().any(|entry| !entry.ok));
        assert!(
            log.iter()
                .any(|entry| entry.tool == "app_state" && !entry.writes)
        );
        assert!(mail.status.starts_with("Agent: Discarded draft"));
        Ok(())
    }

    #[test]
    fn agent_shows_messages_and_requests_fail_without_the_window() -> Result<()> {
        let dir = tempfile::tempdir()?;
        moonmail_store::seed_demo(&mut Archive::open(dir.path())?, &FileBlobs::new(dir.path()))?;
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        let last = mail.page.rows.last().context("rows")?.id;
        let shown = answered(&mut mail, dir.path(), "show_message", json!({"id":last.0}))?;
        assert_eq!(shown["state"], "done");
        assert_eq!(
            mail.selected.as_ref().map(|detail| detail.summary.id),
            Some(last)
        );

        tool(
            dir.path(),
            "create_folder",
            json!({"account":"local","name":"Scratch"}),
        )?;
        let folder = mail
            .archive()?
            .mailboxes()?
            .into_iter()
            .find(|b| b.name == "Scratch")
            .context("folder")?;
        tool(dir.path(), "delete_folder", json!({"id":folder.id}))?;
        assert!(
            !mail
                .archive()?
                .mailboxes()?
                .iter()
                .any(|b| b.id == folder.id)
        );

        // A window that never answers: the request fails and is not left behind.
        drop(mail);
        let started = std::time::Instant::now();
        assert!(tool(dir.path(), "app_state", json!({})).is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(15));
        assert!(Archive::open(dir.path())?.pending_requests()?.is_empty());
        Ok(())
    }

    #[test]
    fn sending_waits_for_the_users_click_on_exactly_what_they_saw() -> Result<()> {
        let dir = tempfile::tempdir()?;
        moonmail_store::seed_demo(&mut Archive::open(dir.path())?, &FileBlobs::new(dir.path()))?;
        let mut mail = Mail::new(dir.path().to_owned(), true);
        mail.initialize()?;
        let made = tool(
            dir.path(),
            "create_draft",
            json!({"to":"ana@example.org","subject":"Lunch","body":"Friday?","account":"you@example.com","open":false}),
        )?;
        let draft = made["id"].as_str().context("draft")?.to_owned();

        // The agent's send waits; the window does not act on it by itself.
        let asked = tool(dir.path(), "send_draft", json!({"id":draft,"wait":0}))?;
        assert_eq!(asked["state"], "pending");
        let request = asked["request"].as_i64().context("request")?;
        mail.outside_changes()?;
        assert_eq!(
            tool(dir.path(), "request_status", json!({"id":request}))?["state"],
            "pending"
        );
        let approval = mail.snapshot()?["agent"]["approvals"][0].clone();
        assert_eq!(
            (approval["id"].as_i64(), approval["subject"].as_str()),
            (Some(request), Some("Lunch"))
        );

        // A queue through app_action is the same request, never a send.
        let queued = tool(
            dir.path(),
            "app_action",
            json!({"action":"queue","args":{"id":draft},"wait":0}),
        );
        assert!(queued.is_ok_and(|value| value["state"] == "pending" && value["kind"] == "send"));

        // The agent cannot answer its own request: not through a UI action,
        // and not by naming the digest, which it can compute.
        let digest = approval["digest"].clone();
        assert!(tool(dir.path(), "app_action", json!({"action":"agent_request","args":{"id":request,"decision":"approve","digest":digest}})).is_err());
        assert!(
            mail.handle(data(
                json!({"action":"agent_request","id":request,"decision":"approve","digest":digest})
            ))
            .is_err()
        );
        assert_eq!(
            tool(dir.path(), "request_status", json!({"id":request}))?["state"],
            "pending"
        );

        // Approving content that changed since it was shown sends nothing.
        let shown = approval["digest"].clone();
        let key = approval["key"].clone();
        mail.archive()?.save_draft(&Draft {
            subject: "Lunch on Friday".into(),
            ..mail.archive()?.draft(&draft)?.context("draft")?
        })?;
        assert!(
            mail.handle(data(
                json!({"action":"agent_request","id":request,"decision":"approve","digest":shown,"key":key})
            ))
            .is_err()
        );
        assert_eq!(
            tool(dir.path(), "request_status", json!({"id":request}))?["state"],
            "failed"
        );

        // An agent edit cancels a waiting send; a denial is final.
        let asked = tool(dir.path(), "send_draft", json!({"id":draft,"wait":0}))?;
        tool(
            dir.path(),
            "update_draft",
            json!({"id":draft,"append":"Noon?"}),
        )?;
        assert_eq!(
            tool(dir.path(), "request_status", json!({"id":asked["request"]}))?["state"],
            "expired"
        );
        let asked = tool(dir.path(), "send_draft", json!({"id":draft,"wait":0}))?;
        mail.handle(data(
            json!({"action":"agent_request","id":asked["request"],"decision":"deny"}),
        ))?;
        assert_eq!(
            tool(dir.path(), "request_status", json!({"id":asked["request"]}))?["state"],
            "denied"
        );

        // Send on what was shown reaches the outbox path (offline in the demo).
        let asked = tool(dir.path(), "send_draft", json!({"id":draft,"wait":0}))?;
        mail.outside_changes()?;
        let pending = mail.snapshot()?["agent"]["approvals"]
            .as_array()
            .map_or(0, Vec::len);
        let card = mail.snapshot()?["agent"]["approvals"][pending - 1].clone();
        assert!(mail.handle(data(json!({"action":"agent_request","id":asked["request"],"decision":"approve","digest":card["digest"],"key":card["key"]}))).is_err());
        let status = tool(dir.path(), "request_status", json!({"id":asked["request"]}))?;
        assert_eq!(
            status["detail"],
            "Sending is disabled in the synthetic demo"
        );
        assert_eq!(mail.archive()?.outbox_count()?, 0);
        Ok(())
    }
}
