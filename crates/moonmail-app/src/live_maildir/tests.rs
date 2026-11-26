use {
    super::*,
    moonmail_core::{MailQuery, MailRepository, MessageId},
    std::{os::unix::fs::PermissionsExt, path::PathBuf},
    tempfile::{TempDir, tempdir},
};

const RAW: &[u8] = b"From: sender@example.test\r\nSubject: Synthetic delivery\r\n\r\nShared body";

fn scan_all(root: &Path) -> Result<bool> {
    let (changed, error) = worker::scan(root, &mut None)?;
    match error {
        Some(e) => anyhow::bail!(e),
        None => Ok(changed),
    }
}

struct Shared {
    store: TempDir,
    source: TempDir,
    account: String,
}
impl Shared {
    fn new() -> Result<Self> {
        let store = tempdir()?;
        let source = tempdir()?;
        for dir in ["cur", "new", "tmp", ".Lists.Rust/cur", ".Lists.Rust/new"] {
            fs::create_dir_all(source.path().join(dir))?;
        }
        attach(store.path(), source.path())?;
        attach(store.path(), source.path())?;
        let accounts = Archive::open(store.path())?.maildir_accounts()?;
        assert_eq!(accounts.len(), 1);
        Ok(Self {
            store,
            source,
            account: accounts[0].account.clone(),
        })
    }
    fn path(&self, name: &str) -> PathBuf {
        self.source.path().join(name)
    }
    fn scan(&self) -> Result<bool> {
        scan_all(self.store.path())
    }
    fn archive(&self) -> Result<Archive> {
        Ok(Archive::open(self.store.path())?)
    }
    fn files(&self) -> Result<Vec<MaildirFile>> {
        Ok(self.archive()?.maildir_files(&self.account)?)
    }
}

#[test]
fn external_delivery_move_flags_and_expunge_survive_reopen() -> Result<()> {
    let shared = Shared::new()?;
    fs::write(shared.path("tmp/inflight"), b"partial")?;
    fs::write(shared.path("new/a,U=23"), RAW)?;
    fs::write(shared.path("new/b"), RAW)?;
    assert!(shared.scan()?);
    assert!(!shared.scan()?);
    let files = shared.files()?;
    assert_eq!(files.len(), 2);
    let first = files
        .iter()
        .find(|f| f.location.stem == "a,U=23")
        .context("first delivery")?;
    let id = MessageId(first.id as u64);
    shared.archive()?.set_flag(id, "flagged", true)?;
    // Mutt changes flags and moves the message while Moonmail is closed.
    fs::rename(
        shared.path("new/a,U=23"),
        shared.path(".Lists.Rust/cur/renamed,U=9:2,RSx,ext"),
    )?;
    let moved = shared.path(".Lists.Rust/cur/renamed,U=9:2,FRSx,ext");
    fs::set_permissions(
        shared.path(".Lists.Rust/cur/renamed,U=9:2,RSx,ext"),
        fs::Permissions::from_mode(0o000),
    )?;
    assert!(shared.scan()?);
    assert!(moved.is_file());
    assert!(!shared.scan()?);
    let files = shared.files()?;
    let updated = files
        .iter()
        .find(|f| f.id == first.id)
        .context("same occurrence")?;
    assert_ne!(updated.mailbox, first.mailbox);
    assert!(updated.seen && updated.starred && updated.want_star.is_none());
    shared.archive()?.set_thread_seen(id, false)?;
    shared.scan()?;
    let unread = shared.path(".Lists.Rust/cur/renamed,U=9:2,FRx,ext");
    assert!(unread.exists());
    fs::set_permissions(&unread, fs::Permissions::from_mode(0o600))?;
    // A copy/unlink move with unchanged unique name verifies the new inode.
    fs::copy(&unread, shared.path("cur/renamed,U=9:2,R"))?;
    fs::remove_file(unread)?;
    shared.scan()?;
    let files = shared.files()?;
    let returned = files
        .iter()
        .find(|f| f.id == first.id)
        .context("copied move")?;
    assert_eq!(returned.mailbox, first.mailbox);
    assert!(!returned.seen && !returned.starred);
    fs::remove_file(shared.path("cur/renamed,U=9:2,R"))?;
    shared.scan()?;
    assert_eq!(shared.files()?.len(), 1);
    assert_eq!(
        shared
            .archive()?
            .page(&MailQuery::default(), 10)?
            .rows
            .len(),
        1
    );
    assert_eq!(shared.archive()?.detail(id)?.text.trim(), "Shared body");
    Ok(())
}

#[test]
fn incomplete_inventory_and_rename_conflicts_preserve_files_and_intent() -> Result<()> {
    let shared = Shared::new()?;
    fs::write(shared.path("new/a"), RAW)?;
    fs::write(shared.path("new/b"), RAW)?;
    shared.scan()?;
    let files = shared.files()?;
    let a = files.iter().find(|f| f.location.stem == "a").context("a")?;
    shared
        .archive()?
        .set_flags(&[MessageId(a.id as u64)], "seen", true)?;
    fs::remove_file(shared.path("new/b"))?;
    fs::remove_dir(shared.path(".Lists.Rust/cur"))?;
    assert!(shared.scan().is_err());
    assert_eq!(shared.files()?.len(), 2);
    fs::create_dir(shared.path(".Lists.Rust/cur"))?;

    let device = identity(&fs::metadata(shared.source.path())?);
    let snapshot = Inventory::read(shared.source.path(), &device)?;
    let entry = &snapshot.entries[0];
    fs::write(shared.path("cur/a:2,S"), b"another delivery")?;
    assert!(write_flags(entry, (true, false)).is_err());
    assert_eq!(fs::read(shared.path("cur/a:2,S"))?, b"another delivery");
    assert!(snapshot.validate().is_err());
    assert!(shared.scan().is_err());
    assert_eq!(shared.files()?.len(), 2);
    assert!(shared.path("new/a").exists());
    fs::remove_file(shared.path("cur/a:2,S"))?;
    fs::rename(shared.path("new/a"), shared.path("cur/a:2,Z"))?;
    assert!(write_flags(entry, (true, false)).is_err());
    shared.scan()?;
    assert_eq!(shared.files()?.len(), 1);
    assert_eq!(fs::read(shared.path("cur/a:2,SZ"))?, RAW);
    Ok(())
}

#[test]
fn hard_linked_deliveries_and_concurrent_clicks_remain_independent() -> Result<()> {
    let shared = Shared::new()?;
    fs::write(shared.path("new/a"), RAW)?;
    fs::hard_link(shared.path("new/a"), shared.path(".Lists.Rust/new/a"))?;
    shared.scan()?;
    let files = shared.files()?;
    assert_eq!(files.len(), 2);
    assert_ne!(files[0].id, files[1].id);
    let id = MessageId(files[0].id as u64);
    shared.archive()?.set_flag(id, "seen", true)?;
    let pending = shared
        .files()?
        .into_iter()
        .find(|f| f.id == files[0].id)
        .context("pending")?;
    shared.archive()?.set_flag(id, "seen", false)?;
    shared
        .archive()?
        .observe_maildir(&pending, pending.mailbox, &pending.location, true, false)?;
    let newer = shared
        .files()?
        .into_iter()
        .find(|f| f.id == pending.id)
        .context("newer")?;
    assert!(!newer.seen);
    assert_eq!(newer.want_seen, Some(false));
    shared.scan()?;
    assert!(
        shared
            .files()?
            .iter()
            .all(|f| !f.seen && f.want_seen.is_none())
    );
    Ok(())
}

#[test]
fn container_restart_and_missing_mailbox_structure() -> Result<()> {
    let store = tempdir()?;
    let tree = tempdir()?;
    for dir in ["Inbox/cur", "Inbox/new", "Trash/cur", "Trash/new"] {
        fs::create_dir_all(tree.path().join(dir))?;
    }
    fs::write(tree.path().join("Inbox/new/message"), RAW)?;
    attach(store.path(), tree.path())?;
    scan_all(store.path())?;
    let mut archive = Archive::open(store.path())?;
    let account = &archive.maildir_accounts()?[0].account;
    let files = archive.maildir_files(account)?;
    let id = MessageId(files[0].id as u64);
    let dest = archive.maildir_folders(account)?["Trash"].0;
    assert!(
        archive
            .queue_moves(&[id], None, moonmail_store::MoveTarget::Folder(dest))
            .is_err()
    );
    assert!(
        archive
            .edit_local_folder(account, moonmail_store::FolderEdit::Delete(dest))
            .is_err()
    );
    fs::remove_dir_all(tree.path().join("Inbox/cur"))?;
    fs::remove_dir_all(tree.path().join("Inbox/new"))?;
    assert!(scan_all(store.path()).is_err());
    assert!(archive.live_summary(id)?.is_some());
    fs::remove_dir(tree.path().join("Inbox"))?;
    assert!(scan_all(store.path())?);
    assert!(archive.live_summary(id)?.is_none());
    assert_eq!(archive.maildir_folders(account)?.len(), 1);
    fs::remove_dir_all(tree.path().join("Trash"))?;
    assert!(scan_all(store.path())?);
    assert!(archive.maildir_folders(account)?.is_empty());
    Ok(())
}

#[test]
fn unavailable_or_replaced_root_keeps_the_queued_edit_bound_to_the_original() -> Result<()> {
    let shared = Shared::new()?;
    fs::write(shared.path("new/a"), RAW)?;
    shared.scan()?;
    let original = shared.files()?.remove(0);
    shared
        .archive()?
        .set_flag(MessageId(original.id as u64), "flagged", true)?;
    // Simulate unmount/remount with another tree at exactly the registered path.
    let backup = tempdir()?;
    let saved = backup.path().join("original");
    fs::rename(shared.source.path(), &saved)?;
    assert!(shared.scan().is_err());
    fs::create_dir_all(shared.path("new"))?;
    fs::create_dir(shared.path("cur"))?;
    fs::write(shared.path("new/a"), RAW)?;
    assert!(shared.scan().is_err());
    assert!(shared.path("new/a").is_file());
    assert_eq!(shared.files()?[0].want_star, Some(true));
    fs::remove_dir_all(shared.source.path())?;
    fs::rename(&saved, shared.source.path())?;
    shared.scan()?;
    assert!(shared.path("cur/a:2,F").is_file());
    assert_eq!(shared.files()?[0].id, original.id);
    Ok(())
}
