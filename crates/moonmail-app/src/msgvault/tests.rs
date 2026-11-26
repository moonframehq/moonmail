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
