use {
    super::*,
    moonmail_network::{LiveAccount, LiveStatus},
    std::collections::BTreeMap,
};

#[derive(Default)]
pub(super) struct Workers {
    accounts: BTreeMap<String, Connection>,
    retiring: Vec<(String, LiveAccount)>,
    paused: bool,
}

impl Drop for Workers {
    fn drop(&mut self) {
        // Start shutdown on every account before joining any one worker.
        for c in self.accounts.values() {
            c.worker.stop();
        }
    }
}

struct Connection {
    account: Account,
    worker: LiveAccount,
    status: LiveStatus,
}

impl Connection {
    fn label(&self) -> String {
        if self.worker.finished() {
            return "Worker stopped".into();
        }
        match &self.status {
            LiveStatus::Syncing => "Syncing".into(),
            LiveStatus::Idle => "Live".into(),
            LiveStatus::Manual => "Manual".into(),
            LiveStatus::Waiting => "Polling".into(),
            LiveStatus::Paused => "Paused".into(),
            LiveStatus::Error {
                error,
                retry: Some(after),
            } => format!("{error} · retry in {}s", after.as_secs()),
            LiveStatus::Error { error, retry: None } => format!("{error} · paused"),
        }
    }
}

impl Workers {
    pub fn configure(
        &mut self,
        root: &std::path::Path,
        accounts: Vec<Account>,
        minutes: u16,
    ) -> Result<()> {
        self.retiring.retain(|(_, worker)| !worker.finished());
        for email in self.accounts.keys().cloned().collect::<Vec<_>>() {
            if self
                .accounts
                .get(&email)
                .is_some_and(|c| !accounts.contains(&c.account) || c.worker.finished())
                && let Some(connection) = self.accounts.remove(&email)
            {
                let worker = connection.worker;
                worker.stop();
                self.retiring.push((email, worker));
            }
        }
        for account in accounts {
            if let Some(connection) = self.accounts.get(&account.email) {
                connection.worker.interval(minutes);
            } else if !self.paused
                && !self
                    .retiring
                    .iter()
                    .any(|(email, _)| *email == account.email)
            {
                let worker = LiveAccount::start(root.to_owned(), account.clone(), minutes)?;
                self.accounts.insert(
                    account.email.clone(),
                    Connection {
                        account,
                        worker,
                        status: LiveStatus::Syncing,
                    },
                );
            }
        }
        Ok(())
    }

    pub fn flush(&self, account: &str) {
        if let Some(connection) = self.accounts.get(account) {
            connection.worker.flush();
        }
    }

    /// Wake every account's worker to upload queued changes.
    pub fn flush_all(&self) {
        for connection in self.accounts.values() {
            connection.worker.flush();
        }
    }

    pub fn pause(&mut self) {
        self.paused = true;
        for connection in self.accounts.values() {
            connection.worker.pause();
        }
    }

    pub fn running(&self) -> bool {
        !self.paused && !self.accounts.is_empty()
    }

    pub fn label(&self) -> String {
        if self.paused {
            return "Sync paused · use Sync to resume".into();
        }
        if self.accounts.len() == 1
            && let Some(c) = self.accounts.values().next()
        {
            return c.label();
        }
        if self.accounts.is_empty() {
            return String::new();
        }
        let live = self
            .accounts
            .values()
            .filter(|c| matches!(c.status, LiveStatus::Idle))
            .count();
        let errors = self
            .accounts
            .values()
            .filter(|c| matches!(c.status, LiveStatus::Error { .. }) || c.worker.finished())
            .count();
        let syncing = self
            .accounts
            .values()
            .filter(|c| matches!(c.status, LiveStatus::Syncing))
            .count();
        let mut summary = vec![format!("{live}/{} live", self.accounts.len())];
        if syncing > 0 {
            summary.push(format!("{syncing} syncing"));
        }
        if errors > 0 {
            summary.push(format!("{errors} need attention"));
        }
        summary.join(" · ")
    }

    pub fn details(&self) -> Vec<String> {
        self.accounts
            .values()
            .map(|c| format!("{} · {}", c.account.email, c.label()))
            .collect()
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        for c in self.accounts.values_mut() {
            if let Some(status) = c.worker.events().last() {
                c.status = status;
                changed = true;
            }
        }
        changed
    }
}

impl Mail {
    pub(super) fn configure_workers(&mut self) -> Result<()> {
        if !self.demo {
            self.workers.configure(
                &self.root,
                Account::load_all(&self.root)?,
                self.preferences.sync_minutes,
            )?;
        }
        Ok(())
    }

    pub(super) fn start_sync(&mut self) -> Result<()> {
        anyhow::ensure!(!self.demo, "Demo mode is offline");
        if self.preferences.sync_paused {
            let mut preferences = self.preferences.clone();
            preferences.sync_paused = false;
            preferences.save(&self.root)?;
            self.preferences = preferences;
        }
        self.workers.paused = false;
        self.configure_workers()?;
        anyhow::ensure!(
            !self.workers.accounts.is_empty(),
            "Connect an account first"
        );
        for connection in self.workers.accounts.values() {
            connection.worker.sync();
        }
        Ok(())
    }

    pub(super) fn poll_workers(&mut self) -> Result<bool> {
        if self
            .workers
            .retiring
            .iter()
            .any(|(_, worker)| worker.finished())
        {
            self.configure_workers()?;
        }
        Ok(self.workers.poll())
    }

    pub(super) fn flush_messages(&self, ids: &[MessageId]) -> Result<()> {
        if self.workers.running() {
            for id in ids {
                self.workers.flush(&self.archive()?.message_account(*id)?);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use {
        super::{super::tests::data, *},
        std::time::Duration,
    };

    #[test]
    fn optimistic_edit_wakes_only_selected_account_and_stop_suppresses_writes() -> Result<()> {
        let dir = tempfile::tempdir()?;
        moonmail_store::seed_demo(&mut Archive::open(dir.path())?, &FileBlobs::new(dir.path()))?;
        let mut mail = Mail::new(dir.path().into(), true);
        mail.initialize()?;
        let selected = mail.selected.as_ref().context("demo selection")?.summary.id;
        let email = mail.archive()?.message_account(selected)?;
        let (attempt, calls) = mpsc::channel();
        for email in [&email, "other@example.org"] {
            let account = Account::fastmail(email)?;
            let attempt = attempt.clone();
            let worker =
                LiveAccount::start_with(dir.path().into(), account.clone(), 0, move |account| {
                    let _ = attempt.send(account.email.clone());
                    Err(moonmail_network::Error::Invalid(
                        "Synthetic login rejection",
                    ))
                })?;
            mail.workers.accounts.insert(
                email.to_owned(),
                Connection {
                    account,
                    worker,
                    status: LiveStatus::Manual,
                },
            );
        }
        let starred = mail.archive()?.detail(selected)?.summary.starred;
        mail.handle(data(json!({"action":"star"})))?;
        assert_ne!(mail.archive()?.detail(selected)?.summary.starred, starred);
        assert_eq!(calls.recv_timeout(Duration::from_secs(2))?, email);
        mail.handle(data(json!({"action":"cancel_sync"})))?;
        mail.handle(data(json!({"action":"star"})))?;
        assert_eq!(mail.archive()?.detail(selected)?.summary.starred, starred);
        assert!(calls.recv_timeout(Duration::from_millis(350)).is_err());
        assert!(!mail.workers.running());
        let mut reopened = Mail::new(dir.path().into(), false);
        reopened.initialize()?;
        assert!(reopened.workers.paused);
        assert!(reopened.workers.accounts.is_empty());
        Ok(())
    }
}
