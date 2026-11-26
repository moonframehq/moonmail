use {
    super::reconcile,
    anyhow::Result,
    moonmail_store::{Archive, FileBlobs},
    std::{
        fs::{self, File},
        path::{Path, PathBuf},
        sync::mpsc,
        time::Duration,
    },
};

pub struct Worker {
    _stop: mpsc::Sender<()>,
    pub events: mpsc::Receiver<Result<(), String>>,
}
impl Worker {
    pub fn start(root: PathBuf) -> Result<Self> {
        let (stop, signal) = mpsc::channel();
        let (send, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("moonmail-maildir".into())
            .spawn(move || {
                let mut last_error = None;
                let mut archive = None;
                loop {
                    let (changed, error) = match scan(&root, &mut archive) {
                        Ok(report) => report,
                        Err(e) => (false, Some(format!("{e:#}"))),
                    };
                    if changed || error != last_error {
                        last_error = error.clone();
                        if send.send(error.map_or(Ok(()), Err)).is_err() {
                            break;
                        }
                    }
                    if !matches!(
                        signal.recv_timeout(Duration::from_secs(2)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            _stop: stop,
            events,
        })
    }
}

pub(super) fn scan(
    root: &Path,
    connection: &mut Option<Archive>,
) -> Result<(bool, Option<String>)> {
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("maildir.lock"))?;
    match lock.try_lock() {
        Ok(()) => {},
        Err(fs::TryLockError::WouldBlock) => return Ok((false, None)),
        Err(fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    let archive = match connection {
        Some(archive) => archive,
        None => connection.insert(Archive::open(root)?),
    };
    let blobs = FileBlobs::new(root);
    let mut changed = false;
    let mut errors = Vec::new();
    for account in archive.maildir_accounts()? {
        match reconcile(archive, &blobs, &account) {
            Ok(work) => changed |= work,
            Err(e) => errors.push(format!("{}: {e:#}", account.account)),
        }
    }
    Ok((changed, (!errors.is_empty()).then(|| errors.join("\n"))))
}
