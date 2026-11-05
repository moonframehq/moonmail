use {
    super::*,
    crate::{Auth, AuthError},
    std::{cell::Cell, io, path::PathBuf, process::Command, rc::Rc, sync::Mutex},
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;
const ACCOUNT: &str = "synthetic@example.org";
// A concurrent fork can briefly inherit another test's locked descriptor until
// exec closes it. Keep process creation separate from drop/reacquire assertions.
static PROCESS_TEST: Mutex<()> = Mutex::new(());

struct SharedStore {
    root: PathBuf,
    accesses: Rc<Cell<usize>>,
}

impl SharedStore {
    fn new(root: &Path) -> Self {
        Self {
            root: root.to_owned(),
            accesses: Rc::new(Cell::new(0)),
        }
    }

    fn check_locked(&self, account: &str) {
        assert!(
            matches!(lock_record(&self.root, account), Err(error) if error.kind() == io::ErrorKind::WouldBlock),
            "credential access must occur while holding the shared account lock"
        );
        self.accesses.set(self.accesses.get() + 1);
    }
}

impl CredentialStore for SharedStore {
    type Error = io::Error;
    type Guard = File;

    fn lock(&self, account: &str) -> io::Result<File> {
        lock_record(&self.root, account)
    }
    fn load(&self, account: &str) -> io::Result<Option<String>> {
        self.check_locked(account);
        match std::fs::read_to_string(self.root.join("record.json")) {
            Ok(record) => Ok(Some(record)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    fn save(&self, account: &str, secret: &str) -> io::Result<()> {
        self.check_locked(account);
        std::fs::write(self.root.join("record.json"), secret)
    }
    fn delete(&self, account: &str) -> io::Result<()> {
        self.check_locked(account);
        std::fs::remove_file(self.root.join("record.json"))
    }
}

#[test]
fn independent_auth_instances_lock_before_any_read_write_or_network() -> Result {
    let _serial = PROCESS_TEST.lock().map_err(|error| error.to_string())?;
    let directory = tempfile::tempdir()?;
    let owner = SharedStore::new(directory.path());
    let contender = SharedStore::new(directory.path());
    let accesses = Rc::clone(&contender.accesses);
    let held = owner.lock(ACCOUNT)?;
    let auth = Auth::new(contender);
    // Including authorize proves contention fails before browser/network work.
    for result in [
        auth.password(ACCOUNT, "synthetic".into()),
        auth.credentials(ACCOUNT).map(|_| ()),
        auth.disconnect(ACCOUNT),
        auth.authorize(ACCOUNT),
    ] {
        assert!(
            matches!(result, Err(AuthError::Credentials(error)) if error.kind() == io::ErrorKind::WouldBlock)
        );
    }
    assert_eq!(
        accesses.get(),
        0,
        "contention must fail before touching credentials"
    );
    // Different credential records are independent, including formerly
    // colliding filename-sanitization inputs and differently cased identities.
    drop(owner.lock("other@example.org")?);
    drop(owner.lock("Synthetic@example.org")?);
    let punctuation = owner.lock("a+b@example.org")?;
    drop(owner.lock("a_b@example.org")?);
    drop(punctuation);
    drop(held);
    auth.password(ACCOUNT, "synthetic".into())
        .map_err(|error| format!("password after release: {error:?}"))?;
    let second = Auth::new(SharedStore::new(directory.path()));
    assert_eq!(
        second
            .credentials(ACCOUNT)
            .map_err(|error| format!("read after password: {error:?}"))?
            .value,
        "synthetic"
    );
    second
        .disconnect(ACCOUNT)
        .map_err(|error| format!("disconnect after read: {error:?}"))?;
    assert!(auth.credentials(ACCOUNT).is_err());
    // The error path above must release its guard too.
    drop(owner.lock(ACCOUNT)?);
    Ok(())
}

fn child(root: &Path, blocked: bool) -> Result {
    let result = Command::new(std::env::current_exe()?)
        .args([
            "--exact",
            "credentials::tests::lock_process_probe",
            "--nocapture",
        ])
        .env("MOONMAIL_TEST_LOCK_ROOT", root)
        .env(
            "MOONMAIL_TEST_LOCK_BLOCKED",
            if blocked { "1" } else { "0" },
        )
        .output()?;
    assert!(
        result.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"));
    Ok(())
}

#[test]
fn lock_process_probe() -> Result {
    let Some(root) = std::env::var_os("MOONMAIL_TEST_LOCK_ROOT") else {
        return Ok(());
    };
    let lock = lock_record(Path::new(&root), ACCOUNT);
    if std::env::var("MOONMAIL_TEST_LOCK_BLOCKED")? == "1" {
        assert!(matches!(lock, Err(error) if error.kind() == io::ErrorKind::WouldBlock));
    } else {
        drop(lock?);
    }
    Ok(())
}

#[test]
fn lock_contends_across_processes_and_keeps_a_stable_inode_after_release() -> Result {
    let _serial = PROCESS_TEST.lock().map_err(|error| error.to_string())?;
    let directory = tempfile::tempdir()?;
    let held = lock_record(directory.path(), ACCOUNT)?;
    child(directory.path(), true)?;
    let paths = std::fs::read_dir(directory.path())?.collect::<io::Result<Vec<_>>>()?;
    assert_eq!(paths.len(), 1);
    let path = paths[0].path();
    drop(held);
    assert!(path.exists(), "released locks must never be unlinked");
    child(directory.path(), false)?;
    // Acquiring via a pre-opened descriptor still contends with a new holder.
    let previous = OpenOptions::new().read(true).write(true).open(&path)?;
    let held = lock_record(directory.path(), ACCOUNT)?;
    assert_eq!(
        previous
            .try_lock_exclusive()
            .err()
            .ok_or("lock unexpectedly acquired")?
            .kind(),
        io::ErrorKind::WouldBlock
    );
    drop(held);
    previous.try_lock_exclusive()?;
    Ok(())
}
