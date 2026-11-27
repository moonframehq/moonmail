use {
    super::*,
    flate2::{Compression, write::ZlibEncoder},
    moonmail_core::MailRepository,
    std::io::Write,
    tempfile::{TempDir, tempdir},
};

/// The subset of msgvault's schema the mirror reads.
const SCHEMA: &str = "
CREATE TABLE sources (id INTEGER PRIMARY KEY, source_type TEXT NOT NULL, identifier TEXT NOT NULL);
CREATE TABLE messages (id INTEGER PRIMARY KEY, source_id INTEGER NOT NULL, message_type TEXT NOT NULL,
    sent_at DATETIME, received_at DATETIME, internal_date DATETIME, is_read BOOLEAN DEFAULT TRUE,
    deleted_at DATETIME, deleted_from_source_at DATETIME);
CREATE TABLE message_raw (message_id INTEGER PRIMARY KEY, raw_data BLOB NOT NULL,
    raw_format TEXT NOT NULL, compression TEXT DEFAULT 'zlib', encryption_version INTEGER DEFAULT 0);
CREATE TABLE labels (id INTEGER PRIMARY KEY, source_id INTEGER, name TEXT NOT NULL, system_role TEXT);
CREATE TABLE message_labels (message_id INTEGER NOT NULL, label_id INTEGER NOT NULL,
    PRIMARY KEY (message_id, label_id));
INSERT INTO sources VALUES (1,'imap','imaps://ada@example.test@imap.example.test:993'),
    (2,'apple_messages','local');
INSERT INTO labels VALUES (1,1,'INBOX',NULL),(2,1,'Sent',NULL),(3,1,'Lists',NULL),(4,1,'Trash',NULL);
";

struct Vault {
    dir: TempDir,
    store: TempDir,
}

impl Vault {
    fn new() -> Result<Self> {
        let vault = Self {
            dir: tempdir()?,
            store: tempdir()?,
        };
        vault.db()?.execute_batch(SCHEMA)?;
        Ok(vault)
    }

    fn path(&self) -> PathBuf {
        self.dir.path().join("msgvault.db")
    }

    fn db(&self) -> Result<Connection> {
        Ok(Connection::open(self.path())?)
    }

    fn add(&self, id: i64, subject: &str, read: bool, labels: &[i64]) -> Result<()> {
        let raw = format!(
            "From: Bob <bob@example.test>\r\nTo: ada@example.test\r\nSubject: {subject}\r\n\
             Message-ID: <{id}@example.test>\r\n\r\nSynthetic body {id}"
        );
        let mut zlib = ZlibEncoder::new(Vec::new(), Compression::default());
        zlib.write_all(raw.as_bytes())?;
        let db = self.db()?;
        db.execute(
            "INSERT INTO messages(id,source_id,message_type,internal_date,is_read)
             VALUES (?1,1,'email','2025-01-02 03:04:05',?2)",
            rusqlite::params![id, read],
        )?;
        db.execute(
            "INSERT INTO message_raw(message_id,raw_data,raw_format) VALUES (?1,?2,'mime')",
            rusqlite::params![id, zlib.finish()?],
        )?;
        for label in labels {
            db.execute("INSERT INTO message_labels VALUES (?1,?2)", [id, *label])?;
        }
        Ok(())
    }

    fn sync(&self) -> Result<usize> {
        let mut archive = Archive::open(self.store.path())?;
        sync(self.store.path(), &mut archive, |_, _| {})
    }

    /// (mailbox name, total, unread) for the mirrored account.
    fn boxes(&self) -> Result<Vec<(String, u64, u64)>> {
        let mut boxes = Archive::open(self.store.path())?
            .mailboxes()?
            .into_iter()
            .filter(|b| b.account == "msgvault ada@example.test" && b.total > 0)
            .map(|b| (b.name, b.total, b.unread))
            .collect::<Vec<_>>();
        boxes.sort();
        Ok(boxes)
    }
}

#[test]
fn detects_email_sources_and_names_them_by_login() -> Result<()> {
    let vault = Vault::new()?;
    vault.add(1, "Hello", true, &[1])?;
    let db = vault.db()?;
    db.execute(
        "INSERT INTO messages(id,source_id,message_type) VALUES (2,2,'imessage')",
        [],
    )?;
    assert_eq!(
        detect(vault.store.path(), &vault.path())?,
        ["msgvault ada@example.test"]
    );
    assert!(detect(vault.store.path(), &vault.path())?.is_empty());
    Ok(())
}

#[test]
fn mirrors_folders_read_state_and_follows_changes() -> Result<()> {
    let vault = Vault::new()?;
    vault.add(1, "Welcome", true, &[1])?;
    vault.add(2, "Unread list post", false, &[1, 3])?;
    vault.add(3, "Reply", true, &[2])?;
    vault.add(4, "No folder", true, &[])?;
    detect(vault.store.path(), &vault.path())?;
    assert_eq!(vault.sync()?, 4);
    assert_eq!(
        vault.boxes()?,
        [
            ("All Mail".into(), 1, 0),
            ("INBOX".into(), 2, 1),
            ("Lists".into(), 1, 1),
            ("Sent".into(), 1, 0),
        ]
    );
    let archive = Archive::open(vault.store.path())?;
    let sent = archive
        .mailboxes()?
        .into_iter()
        .find(|b| b.name == "Sent")
        .context("Sent")?;
    assert_eq!(sent.role, Some(MailboxRole::Sent));
    assert_eq!(archive.total()?, 4);
    drop(archive);

    // Nothing changed: no work.
    assert_eq!(vault.sync()?, 0);

    let db = vault.db()?;
    db.execute("UPDATE messages SET is_read=1 WHERE id=2", [])?;
    db.execute("DELETE FROM message_labels WHERE message_id=1", [])?;
    db.execute("INSERT INTO message_labels VALUES (1,4)", [])?;
    db.execute(
        "UPDATE messages SET deleted_from_source_at='2025-02-01' WHERE id=3",
        [],
    )?;
    vault.add(5, "Late arrival", false, &[1])?;
    assert_eq!(vault.sync()?, 4);
    assert_eq!(
        vault.boxes()?,
        [
            ("All Mail".into(), 1, 0),
            ("INBOX".into(), 2, 1),
            ("Lists".into(), 1, 0),
            ("Trash".into(), 1, 0),
        ]
    );
    // Deleted messages keep their bytes but leave every folder.
    assert_eq!(Archive::open(vault.store.path())?.total()?, 5);
    Ok(())
}

#[test]
fn a_read_state_set_in_moonmail_stays_until_msgvault_changes_it() -> Result<()> {
    let vault = Vault::new()?;
    vault.add(1, "Unread", false, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    vault.sync()?;
    let mut archive = Archive::open(vault.store.path())?;
    let id = archive.msgvault_states("msgvault ada@example.test")?[&1].message;
    archive.set_flags(&[id], "seen", true)?;
    drop(archive);
    vault.sync()?;
    assert_eq!(vault.boxes()?, [("INBOX".into(), 1, 0)]);
    let db = vault.db()?;
    db.execute("UPDATE messages SET is_read=1 WHERE id=1", [])?;
    vault.sync()?;
    db.execute("UPDATE messages SET is_read=0 WHERE id=1", [])?;
    vault.sync()?;
    assert_eq!(vault.boxes()?, [("INBOX".into(), 1, 1)]);
    Ok(())
}

#[test]
fn never_writes_to_msgvault_and_refuses_moves() -> Result<()> {
    let vault = Vault::new()?;
    vault.add(1, "Hello", true, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    let before = fs::read(vault.path())?;
    vault.sync()?;
    assert_eq!(fs::read(vault.path())?, before);
    let archive = Archive::open(vault.store.path())?;
    let inbox = archive
        .mailboxes()?
        .into_iter()
        .find(|b| b.name == "INBOX")
        .context("INBOX")?;
    assert_eq!(inbox.role, Some(MailboxRole::Inbox));
    Ok(())
}

#[test]
fn reads_login_from_imap_identifiers() {
    assert_eq!(
        address("imaps://fabien@pen.so@imap.fastmail.com:993"),
        "fabien@pen.so"
    );
    assert_eq!(address("ada@example.test"), "ada@example.test");
}

#[test]
fn the_worker_rescans_only_after_msgvault_commits() -> Result<()> {
    let vault = Vault::new()?;
    vault.add(1, "Hello", true, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    let (send, events) = mpsc::channel();
    let (mut archive, mut stamps) = (None, HashMap::new());
    assert!(scan(vault.store.path(), &mut archive, &mut stamps, &send)?);
    assert!(!scan(vault.store.path(), &mut archive, &mut stamps, &send)?);
    vault.add(2, "Next", false, &[1])?;
    assert!(scan(vault.store.path(), &mut archive, &mut stamps, &send)?);
    assert!(matches!(events.try_recv(), Ok(Event::Progress(_))));
    assert_eq!(vault.boxes()?, [("INBOX".into(), 2, 1)]);
    Ok(())
}

const ADA: &str = "msgvault ada@example.test";

#[test]
fn imports_newest_first_and_picks_up_arrivals_and_resumes() -> Result<()> {
    let vault = Vault::new()?;
    for id in 1..=450 {
        vault.add(id, &format!("Message {id}"), true, &[1])?;
    }
    detect(vault.store.path(), &vault.path())?;
    // Stop after the first backfill batch, as a quit would.
    let mut archive = Archive::open(vault.store.path())?;
    let db = open(&vault.path())?;
    let account = archive.msgvault_accounts()?.remove(0);
    let mut mirror = Mirror::new(&mut archive, &db, &account)?;
    mirror.archive.begin_msgvault(ADA, 450)?;
    assert_eq!(mirror.batch(451, false)?, Some((251, 200)));
    drop(mirror);
    let states = archive.msgvault_states(ADA)?;
    assert!(states.contains_key(&450) && states.contains_key(&251));
    assert!(!states.contains_key(&250));
    // The newest message was stored first.
    assert!(states[&450].message.0 < states[&251].message.0);
    let account = archive.msgvault_accounts()?.remove(0);
    assert_eq!((account.cursor, account.backfill), (450, 251));
    drop(archive);

    vault.add(451, "Arrived meanwhile", false, &[1])?;
    assert_eq!(vault.sync()?, 251);
    let archive = Archive::open(vault.store.path())?;
    let account = archive.msgvault_accounts()?.remove(0);
    assert_eq!((account.cursor, account.backfill), (451, 0));
    assert_eq!(archive.msgvault_states(ADA)?.len(), 451);
    assert_eq!(vault.boxes()?, [("INBOX".into(), 451, 1)]);
    Ok(())
}

#[test]
fn removed_accounts_disappear_stay_removed_and_can_be_restored() -> Result<()> {
    let vault = Vault::new()?;
    vault.add(1, "Hello", false, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    vault.sync()?;
    let mut archive = Archive::open(vault.store.path())?;
    archive.remove_msgvault(ADA)?;
    assert!(archive.remove_msgvault(ADA).is_err());
    assert!(archive.msgvault_accounts()?.is_empty());
    assert!(archive.mailboxes()?.iter().all(|b| b.account != ADA));
    drop(archive);
    assert!(detect(vault.store.path(), &vault.path())?.is_empty());
    Archive::open(vault.store.path())?.forget_removed_msgvault()?;
    assert_eq!(detect(vault.store.path(), &vault.path())?, [ADA]);
    vault.sync()?;
    assert_eq!(vault.boxes()?, [("INBOX".into(), 1, 1)]);
    Ok(())
}

#[test]
fn a_removal_during_import_is_not_undone_by_the_import() -> Result<()> {
    let vault = Vault::new()?;
    vault.add(1, "Hello", false, &[1])?;
    vault.add(2, "Again", false, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    let mut archive = Archive::open(vault.store.path())?;
    let db = open(&vault.path())?;
    let account = archive.msgvault_accounts()?.remove(0);
    Archive::open(vault.store.path())?.remove_msgvault(ADA)?;
    let mut mirror = Mirror::new(&mut archive, &db, &account)?;
    assert_eq!(mirror.ingest(&mut |_| {})?, 0);
    drop(mirror);
    assert!(archive.mailboxes()?.iter().all(|b| b.account != ADA));
    assert!(archive.msgvault_states(ADA)?.is_empty());
    Ok(())
}

#[test]
fn gmail_stars_follow_the_starred_label() -> Result<()> {
    let vault = Vault::new()?;
    vault
        .db()?
        .execute("INSERT INTO labels VALUES (9,1,'STARRED',NULL)", [])?;
    vault.add(1, "Starred", true, &[1, 9])?;
    detect(vault.store.path(), &vault.path())?;
    vault.sync()?;
    let starred = |vault: &Vault| -> Result<bool> {
        let archive = Archive::open(vault.store.path())?;
        let id = archive.msgvault_states(ADA)?[&1].message;
        Ok(archive.detail(id)?.summary.starred)
    };
    assert!(starred(&vault)?);
    vault
        .db()?
        .execute("DELETE FROM message_labels WHERE label_id=9", [])?;
    vault.sync()?;
    assert!(!starred(&vault)?);
    Ok(())
}

fn files(vault: &Vault) -> usize {
    fn count(dir: &Path) -> usize {
        fs::read_dir(dir).map_or(0, |entries| {
            entries
                .flatten()
                .map(|e| {
                    if e.path().is_dir() {
                        count(&e.path())
                    } else {
                        1
                    }
                })
                .sum()
        })
    }
    count(&vault.store.path().join("messages"))
}

#[test]
fn originals_stay_in_msgvault_and_read_through() -> Result<()> {
    use moonmail_core::BlobStore;
    let vault = Vault::new()?;
    vault.add(1, "Kept in msgvault", true, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    vault.sync()?;
    assert_eq!(files(&vault), 0);
    let archive = Archive::open(vault.store.path())?;
    let id = archive.msgvault_states(ADA)?[&1].message;
    let blobs = FileBlobs::new(vault.store.path());
    let raw = blobs.get(&archive.message_blob(id)?)?;
    assert!(String::from_utf8_lossy(&raw).contains("Subject: Kept in msgvault"));
    assert!(archive.detail(id)?.text.contains("Synthetic body 1"));
    // Gone from msgvault: the message stays listed, its original is unavailable.
    vault
        .db()?
        .execute("DELETE FROM message_raw WHERE message_id=1", [])?;
    assert!(matches!(
        blobs.get(&archive.message_blob(id)?),
        Err(moonmail_store::BlobError::ExternalMissing)
    ));
    Ok(())
}

#[test]
fn earlier_copies_are_released_unless_another_account_uses_them() -> Result<()> {
    use moonmail_core::BlobStore;
    let vault = Vault::new()?;
    vault.add(1, "Only mirrored", true, &[1])?;
    vault.add(2, "Also synced directly", true, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    vault.sync()?;
    let mut archive = Archive::open(vault.store.path())?;
    let blobs = FileBlobs::new(vault.store.path());
    let states = archive.msgvault_states(ADA)?;
    let (only, shared) = (
        archive.message_blob(states[&1].message)?,
        archive.message_blob(states[&2].message)?,
    );
    // Copies written by an earlier version, and one IMAP account sharing a copy.
    for hash in [&only, &shared] {
        let raw = blobs.get(hash)?;
        assert_eq!(&blobs.put(&raw)?, hash);
    }
    let inbox = archive.mailbox("ada@example.test", "INBOX")?;
    archive.ingest(
        &StalwartParser::default(),
        &blobs,
        IncomingMessage {
            account: "ada@example.test",
            mailbox: inbox,
            origin: "imap",
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 0,
            raw: &blobs.get(&shared)?,
        },
    )?;
    assert_eq!(files(&vault), 2);
    assert_eq!(release(&archive, &blobs)?, 1);
    assert_eq!(files(&vault), 1);
    assert!(blobs.get(&only).is_ok() && blobs.get(&shared).is_ok());
    Ok(())
}

#[test]
fn views_across_accounts_show_a_mirrored_message_once() -> Result<()> {
    use moonmail_core::{BlobStore, MailQuery, MailRepository};
    let vault = Vault::new()?;
    vault.add(1, "Synced twice", false, &[1])?;
    vault.add(2, "Only in msgvault", false, &[1])?;
    detect(vault.store.path(), &vault.path())?;
    vault.sync()?;
    let mut archive = Archive::open(vault.store.path())?;
    let blobs = FileBlobs::new(vault.store.path());
    let raw = blobs.get(&archive.message_blob(archive.msgvault_states(ADA)?[&1].message)?)?;
    let inbox = archive.mailbox("ada@example.test", "INBOX")?;
    archive.ingest(
        &StalwartParser::default(),
        &blobs,
        IncomingMessage {
            account: "ada@example.test",
            mailbox: inbox,
            origin: "imap",
            epoch: 0,
            uid: None,
            seen: false,
            starred: false,
            received_at: 0,
            raw: &raw,
        },
    )?;
    let subjects = |query: MailQuery| -> Result<Vec<(String, String)>> {
        let mut rows = archive
            .page(&query, 50)?
            .rows
            .into_iter()
            .map(|row| Ok((row.subject, archive.message_account(row.id)?)))
            .collect::<Result<Vec<_>>>()?;
        rows.sort();
        Ok(rows)
    };
    assert_eq!(
        subjects(MailQuery::default())?,
        [
            ("Only in msgvault".into(), ADA.into()),
            ("Synced twice".into(), "ada@example.test".into()),
        ]
    );
    let mirror = MailQuery {
        account: Some(ADA.into()),
        ..MailQuery::default()
    };
    assert_eq!(subjects(mirror)?.len(), 2);
    Ok(())
}
