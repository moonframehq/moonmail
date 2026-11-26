//! A worker owns one account, its authenticated session and its SQLite connection.
//! Commands are level-triggered: many edits require only one pass over the journal.
use {
    crate::{Account, Auth, Error, Keychain, Result, RustImap},
    imap::types::UnsolicitedResponse,
    moonmail_store::Archive,
    std::{
        path::{Path, PathBuf},
        sync::{
            Arc,
            atomic::{AtomicU8, AtomicU16, Ordering},
            mpsc,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    },
};

const NONE: u8 = 0;
const FLUSH: u8 = 1;
const SYNC: u8 = 2;
const PAUSE: u8 = 3;
const STOP: u8 = 4;

#[derive(Debug)]
pub enum LiveStatus {
    Syncing,
    Idle,
    Waiting,
    Manual,
    Paused,
    Error {
        error: Error,
        retry: Option<Duration>,
    },
}

struct Control {
    command: Arc<AtomicU8>,
    minutes: AtomicU16,
    events: mpsc::Sender<LiveStatus>,
}

pub struct LiveAccount {
    control: Arc<Control>,
    events: mpsc::Receiver<LiveStatus>,
    worker: Option<JoinHandle<()>>,
}

impl LiveAccount {
    pub fn start(root: PathBuf, account: Account, minutes: u16) -> Result<Self> {
        Self::start_with(root, account, minutes, |account| {
            RustImap::connect(account, &Auth::new(Keychain).credentials_for(account)?)
        })
    }

    /// The connector runs on the worker, including credential lookup and TLS.
    pub fn start_with(
        root: PathBuf,
        account: Account,
        minutes: u16,
        connect: impl Fn(&Account) -> Result<RustImap> + Send + 'static,
    ) -> Result<Self> {
        let (events, receiver) = mpsc::channel();
        let control = Arc::new(Control {
            command: Arc::new(AtomicU8::new(if minutes == 0 { NONE } else { SYNC })),
            minutes: AtomicU16::new(minutes),
            events,
        });
        let owner = control.clone();
        let worker = thread::Builder::new()
            .name("moonmail-account".into())
            .spawn(move || owner.run(&root, &account, connect))?;
        Ok(Self {
            control,
            events: receiver,
            worker: Some(worker),
        })
    }

    pub fn stop(&self) {
        self.signal(STOP);
    }
    pub fn finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub fn events(&self) -> impl Iterator<Item = LiveStatus> + '_ {
        self.events.try_iter()
    }
    pub fn sync(&self) {
        self.signal(SYNC);
    }
    pub fn pause(&self) {
        self.signal(PAUSE);
    }
    pub fn flush(&self) {
        if self
            .control
            .command
            .compare_exchange(NONE, FLUSH, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.wake();
        }
    }
    pub fn interval(&self, minutes: u16) {
        if self.control.minutes.swap(minutes, Ordering::AcqRel) != minutes {
            self.flush();
        }
    }
    fn wake(&self) {
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
    }
    fn signal(&self, command: u8) {
        self.control.command.store(command, Ordering::Release);
        self.wake();
    }
}

impl Drop for LiveAccount {
    fn drop(&mut self) {
        self.signal(STOP);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Control {
    fn command(&self) -> u8 {
        self.command.load(Ordering::Acquire)
    }
    fn emit(&self, status: LiveStatus) {
        let _ = self.events.send(status);
    }
    fn run(&self, root: &Path, account: &Account, connect: impl Fn(&Account) -> Result<RustImap>) {
        if self.minutes.load(Ordering::Acquire) == 0 {
            self.emit(LiveStatus::Manual);
            while self.command() == NONE {
                thread::park();
            }
        }
        let mut failures = 0u32;
        loop {
            match self.command() {
                STOP => return,
                PAUSE => {
                    self.emit(LiveStatus::Paused);
                    while self.command() == PAUSE {
                        thread::park();
                    }
                    continue;
                },
                SYNC => failures = 0,
                _ => {},
            }
            if let Err(error) = self.connected(root, account, &connect, &mut failures) {
                if self.command() >= PAUSE {
                    continue;
                }
                let retry = error.retryable_sync().then(|| reconnect_delay(failures));
                failures = failures.saturating_add(1);
                self.emit(LiveStatus::Error { error, retry });
                let deadline = retry.map(|d| Instant::now() + d);
                while self.command() < SYNC {
                    if let Some(end) = deadline {
                        if Instant::now() >= end {
                            break;
                        }
                        thread::park_timeout(end.saturating_duration_since(Instant::now()));
                    } else {
                        thread::park();
                    }
                }
            }
        }
    }

    fn connected_jmap(&self, root: &Path, account: &Account, failures: &mut u32) -> Result<()> {
        let secret = Auth::new(Keychain).credentials_for(account)?;
        let client = Arc::new(crate::jmap::Client::fastmail(secret.value)?);
        let mut archive = Archive::open(root)?;
        let blobs = moonmail_store::FileBlobs::new(root);
        let mut watcher = None;
        loop {
            if self
                .command
                .try_update(Ordering::AcqRel, Ordering::Acquire, |c| {
                    (c < PAUSE).then_some(NONE)
                })
                .is_err()
            {
                return Ok(());
            }
            self.emit(LiveStatus::Syncing);
            client.sync(
                &account.email,
                &mut archive,
                &blobs,
                |_| {},
                || self.command() >= PAUSE,
            )?;
            *failures = 0;
            let minutes = self.minutes.load(Ordering::Acquire);
            if minutes == 0 {
                watcher = None;
                self.emit(LiveStatus::Manual);
                while self.command() == NONE {
                    thread::park();
                }
            } else {
                if watcher.is_none() {
                    watcher = Some(crate::jmap_push::listen(&client, {
                        let command = self.command.clone();
                        let worker = thread::current();
                        move || {
                            if command
                                .compare_exchange(NONE, SYNC, Ordering::AcqRel, Ordering::Acquire)
                                .is_ok()
                            {
                                worker.unpark();
                            }
                        }
                    })?);
                }
                self.emit(LiveStatus::Waiting);
                let end = Instant::now() + Duration::from_secs(u64::from(minutes) * 60);
                while self.command() == NONE && Instant::now() < end {
                    thread::park_timeout(end.saturating_duration_since(Instant::now()));
                }
            }
        }
    }

    fn connected(
        &self,
        root: &Path,
        account: &Account,
        connect: &impl Fn(&Account) -> Result<RustImap>,
        failures: &mut u32,
    ) -> Result<()> {
        self.command
            .try_update(Ordering::AcqRel, Ordering::Acquire, |c| {
                (c < PAUSE).then_some(NONE)
            })
            .ok();
        if self.command() >= PAUSE {
            return Ok(());
        }
        self.emit(LiveStatus::Syncing);
        if account.transport == crate::Transport::Jmap {
            return self.connected_jmap(root, account, failures);
        }
        let mut imap = connect(account)?;
        let idle = imap.session.capabilities()?.has_str("IDLE");
        let mut archive = Archive::open(root)?;
        archive.invalidate_jmap(&account.email)?;
        loop {
            if self.command() == FLUSH {
                thread::sleep(Duration::from_millis(200));
            }
            let command = self
                .command
                .try_update(Ordering::AcqRel, Ordering::Acquire, |c| {
                    (c < PAUSE).then_some(NONE)
                });
            if command.is_err() {
                return Ok(());
            }
            self.emit(LiveStatus::Syncing);
            let result = crate::sync::sync_session(
                root,
                account,
                &mut imap,
                &mut archive,
                |_| {},
                || self.command() >= PAUSE,
                || self.command() == FLUSH,
            );
            if matches!(result, Err(Error::Cancelled)) {
                continue;
            }
            let inbox_selected = result?;
            let mut changed = imap.session.take_all_unsolicited().any(mailbox_changed);
            if archive.has_queued_keywords(&account.email)? {
                continue;
            }
            *failures = 0;
            let minutes = self.minutes.load(Ordering::Acquire);
            if minutes == 0 {
                self.emit(LiveStatus::Manual);
                while self.command() == NONE {
                    thread::park();
                }
                continue;
            }
            let interval = Duration::from_secs(u64::from(minutes) * 60);
            if idle && let Some(inbox) = inbox_selected {
                let end = Instant::now() + interval;
                while self.command() == NONE && Instant::now() < end {
                    if changed {
                        self.emit(LiveStatus::Syncing);
                        match crate::sync::sync_folder(
                            &mut imap,
                            account,
                            &mut archive,
                            &moonmail_store::FileBlobs::new(root),
                            &inbox,
                            |_| {},
                            || self.command() != NONE,
                        ) {
                            Ok((0, _)) => {},
                            Ok((n, _)) => return Err(Error::IncompleteSync(n)),
                            Err(crate::SyncError::Cancelled) => break,
                            Err(e) => return Err(e.into()),
                        }
                        changed = imap.session.take_all_unsolicited().any(mailbox_changed);
                        if archive.has_queued_keywords(&account.email)? {
                            break;
                        }
                        continue;
                    }
                    self.emit(LiveStatus::Idle);
                    let renew = end.min(Instant::now() + Duration::from_secs(20 * 60));
                    changed = imap.wire.idle(renew, || self.command() != NONE)?;
                }
            } else {
                self.emit(LiveStatus::Waiting);
                let end = Instant::now() + interval;
                while self.command() == NONE && Instant::now() < end {
                    thread::park_timeout(end.saturating_duration_since(Instant::now()));
                }
            }
        }
    }
}

fn mailbox_changed(response: UnsolicitedResponse) -> bool {
    matches!(
        response,
        UnsolicitedResponse::Exists(_)
            | UnsolicitedResponse::Expunge(_)
            | UnsolicitedResponse::Fetch { .. }
            | UnsolicitedResponse::Bye { .. }
    )
}

fn reconnect_delay(failures: u32) -> Duration {
    Duration::from_secs((30 << failures.min(5)).min(900))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reconnect_delay_is_exponential_and_capped() {
        assert_eq!(
            [0, 1, 2, 3, 4, 5, 6, u32::MAX].map(|n| reconnect_delay(n).as_secs()),
            [30, 60, 120, 240, 480, 900, 900, 900]
        );
    }
}
