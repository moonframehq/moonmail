use {
    moonmail_core::{MailQuery, MailRepository},
    moonmail_network::{
        Account, CopyOutcome, IncomingMail, KeywordOutcome, RemoteFolder, RemoteMessage,
        RemoteSnapshot, synchronize, synchronize_cancellable,
    },
    moonmail_store::{Archive, FileBlobs, MoveTarget},
    std::{
        cell::Cell,
        collections::{BTreeMap, BTreeSet},
        io,
        rc::Rc,
    },
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn downloaded_entry_rules_queue_once_and_following_sync_uploads_their_keyword_intent() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.org")?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox(&account.email, "INBOX")?;
    archive.save_keyword_rule(&moonmail_store::KeywordRule {
        id: 0,
        revision: 0,
        mailbox: inbox,
        name: "New entry".into(),
        source: "keyword:remote".into(),
        predicate: moonmail_core::MailPredicate::Keyword("remote".into()),
        keyword: "work".into(),
        value: true,
        enabled: true,
    })?;
    let mut server = Server::new();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 1);
    assert_eq!(archive.keyword_operations(&account.email, 0)?.len(), 1);
    assert_eq!(
        server.stores, 0,
        "new-entry intent waits for the following sync"
    );
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.stores, 1);
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 1);
    server.epoch = 8;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(
        archive.keyword_rule_runs(None)?.len(),
        2,
        "replacement UID namespace is a new entry"
    );
    assert_eq!(archive.keyword_operations(&account.email, 0)?[0].epoch, 8);
    Ok(())
}
struct Server {
    selected: String,
    epoch: u32,
    keywords: BTreeMap<String, BTreeMap<u32, BTreeSet<String>>>,
    stores: usize,
    copies: usize,
    lost: bool,
    unsupported: bool,
    stop: Option<Rc<Cell<bool>>>,
}
impl Server {
    fn new() -> Self {
        Self {
            selected: String::new(),
            epoch: 7,
            keywords: BTreeMap::from([
                (
                    "INBOX".into(),
                    BTreeMap::from([(1, BTreeSet::from(["remote".into()]))]),
                ),
                ("Other".into(), BTreeMap::new()),
            ]),
            stores: 0,
            copies: 0,
            lost: false,
            unsupported: false,
            stop: None,
        }
    }
}
impl IncomingMail for Server {
    type Error = io::Error;
    fn folders(&mut self) -> io::Result<Vec<RemoteFolder>> {
        Ok(self
            .keywords
            .keys()
            .map(|name| RemoteFolder {
                name: name.clone(),
                delimiter: Some("/".into()),
                selectable: true,
                role: None,
            })
            .collect())
    }
    fn snapshot(&mut self, folder: &str) -> io::Result<RemoteSnapshot> {
        self.selected = folder.into();
        Ok(RemoteSnapshot {
            epoch: self.epoch,
            messages: self
                .keywords
                .get(folder)
                .ok_or_else(|| io::Error::other("folder missing"))?
                .iter()
                .map(|(uid, k)| RemoteMessage {
                    uid: *uid,
                    keywords: k.iter().cloned().collect(),
                    seen: false,
                    starred: false,
                    size: 100,
                    received_at: 1,
                })
                .collect(),
        })
    }
    fn fetch(&mut self, _: u32) -> io::Result<Option<Vec<u8>>> {
        Ok(Some(
            b"From: peer@example.org\r\nSubject: Synthetic keywords\r\n\r\nBody".to_vec(),
        ))
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> io::Result<()> {
        Ok(())
    }
    fn keyword(
        &mut self,
        folder: &str,
        epoch: u32,
        uid: u32,
        keyword: &str,
        value: bool,
    ) -> io::Result<KeywordOutcome> {
        self.stores += 1;
        if epoch != self.epoch {
            return Ok(KeywordOutcome::Stale);
        }
        if self.unsupported {
            return Ok(KeywordOutcome::Unsupported);
        }
        let k = self
            .keywords
            .get_mut(folder)
            .and_then(|m| m.get_mut(&uid))
            .ok_or_else(|| io::Error::other("missing source"))?;
        if value {
            k.insert(keyword.into());
        } else {
            k.remove(keyword);
        }
        if let Some(stop) = &self.stop {
            stop.set(true);
        }
        if self.lost {
            return Err(io::Error::other("STORE applied, response lost"));
        }
        Ok(KeywordOutcome::Applied)
    }
    fn copy_message(&mut self, uid: u32, destination: &str) -> io::Result<CopyOutcome> {
        let k = self
            .keywords
            .get(&self.selected)
            .and_then(|m| m.get(&uid))
            .cloned()
            .ok_or_else(|| io::Error::other("missing source"))?;
        assert!(
            k.contains("work"),
            "earlier keyword intent must reach server before COPY"
        );
        self.copies += 1;
        self.keywords
            .get_mut(destination)
            .ok_or_else(|| io::Error::other("missing destination"))?
            .insert(10, k);
        Ok(CopyOutcome::Copied {
            epoch: self.epoch,
            uid: 10,
        })
    }
}

#[test]
fn keyword_sync_retries_idempotently_recovers_restart_and_orders_before_transfers() -> Result {
    for scenario in [
        "success",
        "lost",
        "unsupported",
        "epoch",
        "cancel",
        "ack_failure",
    ] {
        let dir = tempfile::tempdir()?;
        let blobs = FileBlobs::new(dir.path());
        let account = Account::fastmail("me@example.org")?;
        let mut archive = Archive::open(dir.path())?;
        let mut server = Server::new();
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        let id = archive.page(&MailQuery::default(), 10)?.rows[0].id;
        assert_eq!(archive.message_keywords(id)?, ["remote"]);
        let source = archive.mailbox(&account.email, "INBOX")?;
        let dest = archive.mailbox(&account.email, "Other")?;
        archive.set_keywords(&[id], "Work", true)?;
        archive.queue_copies(&[id], Some(source), MoveTarget::Folder(dest))?;
        let stop = Rc::new(Cell::new(false));
        if scenario == "cancel" {
            server.stop = Some(stop.clone());
        }
        server.lost = scenario == "lost";
        server.unsupported = scenario == "unsupported";
        if scenario == "epoch" {
            server.epoch = 8;
        }
        let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
        if scenario == "ack_failure" {
            db.execute_batch("CREATE TRIGGER fail_ack BEFORE DELETE ON keyword_operations BEGIN SELECT RAISE(ABORT,'synthetic ack failure'); END;")?;
        }
        let result = synchronize_cancellable(
            &mut server,
            &account,
            &mut archive,
            &blobs,
            |_| {},
            || stop.get(),
        );
        assert_eq!(
            result.is_err(),
            matches!(scenario, "lost" | "cancel" | "ack_failure"),
            "{scenario}"
        );
        if scenario == "ack_failure" {
            db.execute_batch("DROP TRIGGER fail_ack;")?;
        }
        if matches!(scenario, "unsupported" | "epoch") {
            assert_eq!(server.copies, 0);
            assert_eq!(
                archive.keyword_operations(&account.email, 0)?[0].state,
                "blocked"
            );
            synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
            assert_eq!(server.stores, 1, "blocked edits do not spin");
            let op = archive.keyword_operations(&account.email, 0)?[0].id;
            if scenario == "unsupported" {
                assert_eq!(archive.message_keywords(id)?, ["remote", "work"]);
                archive.retry_keyword(op)?;
                server.unsupported = false;
                synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
                assert!(archive.keyword_operations(&account.email, 0)?.is_empty());
            } else {
                archive.discard_keyword(op)?;
            }
            continue;
        }
        if scenario == "cancel" {
            assert!(
                archive.keyword_operations(&account.email, 0)?.is_empty(),
                "ack recorded before cancellation"
            );
        }
        drop(archive);
        let mut archive = Archive::open(dir.path())?;
        server.lost = false;
        server.stop = None;
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        assert_eq!(server.copies, 1);
        assert_eq!(
            server.stores,
            if matches!(scenario, "lost" | "ack_failure") {
                2
            } else {
                1
            }
        );
        assert!(archive.keyword_operations(&account.email, 0)?.is_empty());
        let copies = archive
            .page(
                &MailQuery {
                    mailbox: Some(dest),
                    ..Default::default()
                },
                10,
            )?
            .rows;
        assert_eq!(archive.message_keywords(copies[0].id)?, ["remote", "work"]);
        assert_eq!(archive.message_keywords(id)?, ["remote", "work"]);
        archive.set_keywords(&[id], "work", false)?;
        server.lost = true;
        assert!(synchronize(&mut server, &account, &mut archive, &blobs, |_| {}).is_err());
        server.lost = false;
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        assert_eq!(archive.message_keywords(id)?, ["remote"]);
        assert_eq!(
            archive.message_keywords(copies[0].id)?,
            ["remote", "work"],
            "membership is occurrence-scoped"
        );
    }
    Ok(())
}
