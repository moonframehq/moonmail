use {
    moonmail_core::{MailQuery, MailRepository},
    moonmail_network::{
        Account, IncomingMail, RemoteFolder, RemoteMessage, RemoteSnapshot, synchronize,
    },
    moonmail_store::{Archive, FileBlobs},
    std::error::Error,
};

struct Fake {
    fail: bool,
}

struct Controlled {
    stop: std::rc::Rc<std::cell::Cell<bool>>,
    cancel_list: bool,
    oversized: bool,
}

impl IncomingMail for Controlled {
    type Error = std::io::Error;
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, Self::Error> {
        let folders = Fake { fail: false }.folders()?;
        if self.cancel_list {
            self.stop.set(true);
        }
        Ok(folders)
    }
    fn snapshot(&mut self, folder: &str) -> Result<RemoteSnapshot, Self::Error> {
        let mut snapshot = Fake { fail: false }.snapshot(folder)?;
        if self.oversized {
            snapshot.messages[0].size = 64 * 1024 * 1024 + 1;
        }
        Ok(snapshot)
    }
    fn fetch(&mut self, uid: u32) -> Result<Option<Vec<u8>>, Self::Error> {
        assert!(
            !self.oversized || uid != 1,
            "oversized body must not be fetched"
        );
        Fake { fail: false }.fetch(uid)
    }
    fn flag(&mut self, uid: u32, kind: &str, value: bool) -> Result<(), Self::Error> {
        Fake { fail: false }.flag(uid, kind, value)
    }
}

#[test]
fn cancellation_retains_durable_downloads_and_does_not_retire_folders_after_cancelled_list()
-> Result<(), Box<dyn Error>> {
    use moonmail_network::{SyncError, synchronize_cancellable};
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.com")?;
    archive.mailbox(&account.email, "Retain until complete")?;
    let stop = std::rc::Rc::new(std::cell::Cell::new(false));
    let mut server = Controlled {
        stop: stop.clone(),
        cancel_list: true,
        oversized: false,
    };
    let result = synchronize_cancellable(
        &mut server,
        &account,
        &mut archive,
        &blobs,
        |_| {},
        || stop.get(),
    );
    assert!(matches!(result, Err(SyncError::Cancelled)));
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.name == "Retain until complete")
    );
    assert_eq!(archive.total()?, 0);
    stop.set(false);
    server.cancel_list = false;
    let result = synchronize_cancellable(
        &mut server,
        &account,
        &mut archive,
        &blobs,
        |status| {
            if status.starts_with("Downloaded") {
                stop.set(true);
            }
        },
        || stop.get(),
    );
    assert!(matches!(result, Err(SyncError::Cancelled)));
    assert_eq!(archive.total()?, 1);
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    stop.set(false);
    synchronize_cancellable(
        &mut server,
        &account,
        &mut archive,
        &blobs,
        |_| {},
        || stop.get(),
    )?;
    assert_eq!(archive.total()?, 2);
    Ok(())
}

#[test]
fn oversized_messages_report_incomplete_without_fetching_or_skipping_other_durable_work()
-> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.com")?;
    let mut server = Controlled {
        stop: Default::default(),
        cancel_list: false,
        oversized: true,
    };
    for _ in 0..2 {
        assert!(matches!(
            synchronize(&mut server, &account, &mut archive, &blobs, |_| {}),
            Err(moonmail_network::SyncError::Incomplete(1))
        ));
        assert_eq!(archive.total()?, 1);
    }
    server.oversized = false;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(archive.total()?, 2);
    Ok(())
}
impl IncomingMail for Fake {
    type Error = std::io::Error;
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, Self::Error> {
        Ok(vec![
            RemoteFolder {
                name: "Parent".into(),
                delimiter: Some(".".into()),
                selectable: false,
                role: None,
            },
            RemoteFolder {
                delimiter: Some("/".into()),
                selectable: true,
                role: Some(moonmail_core::MailboxRole::Inbox),
                name: "INBOX".into(),
            },
        ])
    }
    fn snapshot(&mut self, folder: &str) -> Result<RemoteSnapshot, Self::Error> {
        assert_ne!(folder, "Parent", "NOSELECT folders must not be selected");
        Ok(RemoteSnapshot {
            epoch: 7,
            messages: (1..=2)
                .map(|uid| RemoteMessage {
                    keywords: Vec::new(),
                    uid,
                    seen: false,
                    starred: false,
                    size: 100,
                    received_at: 1_700_000_000 + i64::from(uid),
                })
                .collect(),
        })
    }
    fn fetch(&mut self, uid: u32) -> Result<Option<Vec<u8>>, Self::Error> {
        if self.fail && uid == 1 {
            return Err(std::io::Error::other("synthetic disconnect"));
        }
        Ok(Some(format!("From: sender@example.org\r\nTo: me@example.com\r\nSubject: {uid}\r\n\r\nSynthetic body").into_bytes()))
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn interrupted_sync_resumes_without_skipping_or_duplicating_messages() -> Result<(), Box<dyn Error>>
{
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.com")?;
    assert!(
        synchronize(
            &mut Fake { fail: true },
            &account,
            &mut archive,
            &blobs,
            |_| {}
        )
        .is_err()
    );
    assert_eq!(archive.total()?, 1);
    synchronize(
        &mut Fake { fail: false },
        &account,
        &mut archive,
        &blobs,
        |_| {},
    )?;
    assert_eq!(archive.total()?, 2);
    synchronize(
        &mut Fake { fail: false },
        &account,
        &mut archive,
        &blobs,
        |_| {},
    )?;
    assert_eq!(archive.total()?, 2);
    assert_eq!(archive.page(&MailQuery::default(), 100)?.rows.len(), 2);
    for row in archive.page(&MailQuery::default(), 100)?.rows {
        let uid: i64 = row.subject.parse()?;
        assert_eq!(row.date, 1_700_000_000 + uid);
        let detail = archive.detail(row.id)?;
        assert_eq!(detail.received_at, Some(1_700_000_000 + uid));
        assert!(detail.header_date.is_none());
    }
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.name == "Parent"
                && folder.delimiter.as_deref() == Some(".")
                && !folder.selectable)
    );
    Ok(())
}

struct Listing {
    names: Vec<&'static str>,
    fail_list: bool,
    fail_select: bool,
    epoch: u32,
    flags_written: usize,
}

impl Listing {
    fn new(names: Vec<&'static str>) -> Self {
        Self {
            names,
            fail_list: false,
            fail_select: false,
            epoch: 7,
            flags_written: 0,
        }
    }
}

impl IncomingMail for Listing {
    type Error = std::io::Error;
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, Self::Error> {
        if self.fail_list {
            return Err(std::io::Error::other(
                "LIST interrupted after partial responses",
            ));
        }
        Ok(self
            .names
            .iter()
            .map(|name| RemoteFolder {
                name: (*name).into(),
                delimiter: Some("/".into()),
                selectable: *name != "Parent",
                role: None,
            })
            .collect())
    }
    fn snapshot(&mut self, name: &str) -> Result<RemoteSnapshot, Self::Error> {
        if self.fail_select {
            return Err(std::io::Error::other("SELECT failed"));
        }
        let mut snapshot = Fake { fail: false }.snapshot(name)?;
        snapshot.epoch = self.epoch;
        Ok(snapshot)
    }
    fn fetch(&mut self, uid: u32) -> Result<Option<Vec<u8>>, Self::Error> {
        Fake { fail: false }.fetch(uid)
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> Result<(), Self::Error> {
        self.flags_written += 1;
        Ok(())
    }
}

#[test]
fn complete_lists_retire_only_missing_account_folders_and_preserve_archived_bytes()
-> Result<(), Box<dyn Error>> {
    use moonmail_core::BlobStore;
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.com")?;
    let other = Account::fastmail("other@example.com")?;
    synchronize(
        &mut Listing::new(vec!["INBOX", "Gone", "Parent"]),
        &account,
        &mut archive,
        &blobs,
        |_| {},
    )?;
    synchronize(
        &mut Listing::new(vec!["INBOX"]),
        &other,
        &mut archive,
        &blobs,
        |_| {},
    )?;
    let gone = archive
        .mailboxes()?
        .into_iter()
        .find(|folder| folder.name == "Gone")
        .ok_or("missing folder")?
        .id;
    let query = MailQuery {
        mailbox: Some(gone),
        ..Default::default()
    };
    let old = archive.page(&query, 10)?.rows[0].id;
    let blob = archive.detail(old)?.blob;
    archive.set_flag(old, "seen", true)?;
    let mut listing = Listing::new(vec!["INBOX", "Parent"]);
    listing.fail_list = true;
    assert!(synchronize(&mut listing, &account, &mut archive, &blobs, |_| {}).is_err());
    assert_eq!(archive.page(&query, 10)?.rows.len(), 2);
    assert_eq!(archive.pending_flags(&account.email)?.len(), 1);
    // A malformed complete list must also roll back all local changes.
    assert!(
        archive
            .reconcile_mailboxes(&account.email, ["New", "New"])
            .is_err()
    );
    assert!(
        archive
            .mailboxes()?
            .iter()
            .all(|folder| folder.name != "New")
    );
    assert_eq!(archive.page(&query, 10)?.rows.len(), 2);
    listing.fail_list = false;
    listing.fail_select = true;
    assert!(synchronize(&mut listing, &account, &mut archive, &blobs, |_| {}).is_err());
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert!(archive.page(&query, 10)?.rows.is_empty());
    assert!(archive.pending_flags(&account.email)?.is_empty());
    assert!(
        archive
            .mailboxes()?
            .iter()
            .all(|folder| folder.name != "Gone")
    );
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.name == "Parent" && !folder.selectable)
    );
    assert_eq!(archive.total()?, 6, "original messages remain archived");
    assert!(blobs.get(&blob)?.starts_with(b"From:"));
    assert_eq!(archive.detail(old)?.blob, blob);
    let mut reappeared = Listing::new(vec!["INBOX", "Gone", "Parent"]);
    reappeared.epoch = 8;
    synchronize(&mut reappeared, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(
        reappeared.flags_written, 0,
        "retired UID operations never replay"
    );
    assert!(archive.mailboxes()?.iter().any(|folder| folder.id == gone));
    assert!(
        archive
            .page(&query, 10)?
            .rows
            .iter()
            .all(|row| row.id != old)
    );
    synchronize(
        &mut Listing::new(Vec::new()),
        &account,
        &mut archive,
        &blobs,
        |_| {},
    )?;
    let folders = archive.mailboxes()?;
    assert!(folders.iter().all(|folder| folder.account == other.email));
    assert_eq!(archive.page(&MailQuery::default(), 10)?.rows.len(), 2);
    Ok(())
}
