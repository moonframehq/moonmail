use {
    moonmail_core::{MailRepository, MessageId},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage, MoveTarget},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn bulk_flags_commit_all_occurrences_and_remote_intents_or_roll_back_every_message() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let source = archive.mailbox("a", "Source")?;
    let destination = archive.mailbox("a", "Destination")?;
    let mut ids = Vec::new();
    for n in 1..=2 {
        ids.push(
            archive.ingest(
                &StalwartParser::default(),
                &FileBlobs::new(dir.path()),
                IncomingMessage {
                    account: "a",
                    mailbox: source,
                    origin: &format!("imap:{n}"),
                    epoch: 7,
                    uid: Some(n),
                    seen: false,
                    starred: false,
                    received_at: i64::from(n),
                    raw: format!("From: peer@example.org\r\nSubject: Flag {n}\r\n\r\nSynthetic")
                        .as_bytes(),
                },
            )?,
        );
    }
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute("INSERT INTO occurrences(mailbox_id,message_id,epoch,uid,seen,flagged) VALUES (?1,?2,8,9,0,0)",rusqlite::params![destination,ids[0].0 as i64])?;
    archive.queue_copies(&[ids[1]], Some(source), MoveTarget::Folder(destination))?;
    assert!(archive.set_flags(&ids, "seen", true).is_err());
    assert!(archive.pending_flags("a")?.is_empty());
    assert!(archive.detail(ids[0])?.summary.unread);
    let transfer = archive.move_operations()?[0].id;
    archive.cancel_move(transfer)?;
    assert!(
        archive
            .set_flags(&[ids[0], MessageId(9999)], "seen", true)
            .is_err()
    );
    assert!(archive.pending_flags("a")?.is_empty());
    db.execute_batch("CREATE TRIGGER fail_bulk_flag BEFORE INSERT ON flag_operations WHEN NEW.uid=2 BEGIN SELECT RAISE(ABORT,'synthetic flag journal failure'); END;")?;
    assert!(archive.set_flags(&ids, "seen", true).is_err());
    assert!(archive.pending_flags("a")?.is_empty());
    assert!(archive.detail(ids[0])?.summary.unread);
    db.execute_batch("DROP TRIGGER fail_bulk_flag;")?;
    assert_eq!(
        archive.set_flags(&[ids[0], ids[1], ids[0]], "seen", true)?,
        2
    );
    assert_eq!(
        archive.pending_flags("a")?.len(),
        3,
        "all remote occurrences are queued once"
    );
    archive.set_flags(&ids, "flagged", true)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert_eq!(archive.pending_flags("a")?.len(), 6);
    for id in &ids {
        let summary = archive.detail(*id)?.summary;
        assert!(!summary.unread && summary.starred);
    }
    archive.set_flags(&ids, "seen", false)?;
    assert!(
        archive
            .pending_flags("a")?
            .iter()
            .filter(|flag| flag.kind == "seen")
            .all(|flag| !flag.value)
    );
    assert!(archive.set_flags(&[], "seen", true).is_err());
    assert!(archive.set_flags(&vec![ids[0]; 101], "seen", true).is_err());
    assert!(archive.set_flags(&ids, "deleted", true).is_err());
    Ok(())
}
