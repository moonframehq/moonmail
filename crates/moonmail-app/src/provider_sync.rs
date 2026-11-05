use super::*;

impl Mail {
    pub(super) fn start_sync(&mut self, manual: bool) -> Result<()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        self.launch(move |root, sender| {
            Ok(moonmail_network::sync_account_cancellable(
                &root,
                |status| {
                    let _ = sender.try_send(Event::Progress(status));
                },
                || worker_cancel.load(Ordering::Relaxed),
            )?)
        })?;
        self.sync_cancel = Some(cancel);
        self.sync_schedule.started(manual);
        self.error.clear();
        Ok(())
    }

    pub(super) fn maybe_auto_sync(&mut self, now: std::time::Instant) -> Result<bool> {
        if self.demo || self.job.is_some() || !self.sync_schedule.due(now) {
            return Ok(false);
        }
        if Account::load(&self.root)?.is_none() {
            self.sync_schedule.defer(now);
            return Ok(false);
        }
        self.start_sync(false)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::time::{Duration, Instant},
    };
    #[test]
    fn automatic_sync_never_starts_in_demo_without_account_or_while_worker_is_busy() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let mut mail = Mail::new(dir.path().to_owned(), true);
        let now = Instant::now();
        let due = now + Duration::from_secs(300);
        mail.sync_schedule.configure(5, now);
        assert!(!mail.maybe_auto_sync(due)?);
        assert!(mail.job.is_none());
        mail.demo = false;
        assert!(!mail.maybe_auto_sync(due)?);
        assert!(!mail.sync_schedule.due(due));
        let (_sender, receiver) = mpsc::sync_channel(1);
        mail.job = Some(receiver);
        assert!(!mail.maybe_auto_sync(due + Duration::from_secs(300))?);
        let cancel = Arc::new(AtomicBool::new(false));
        mail.sync_cancel = Some(cancel.clone());
        mail.handle(super::super::tests::data(json!({"action":"cancel_sync"})))?;
        assert!(cancel.load(Ordering::Relaxed));
        mail.sync_schedule.finished(true, false, due);
        assert!(!mail.sync_schedule.due(due + Duration::from_secs(10000)));
        Ok(())
    }
}
