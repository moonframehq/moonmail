use {
    moonmail_core::{MailQuery, MailRepository, MessageId, canonical_keyword},
    moonmail_mime::StalwartParser,
    moonmail_search::{SqliteIndex, parse_query},
    moonmail_store::{Archive, FileBlobs, IncomingMessage, MoveTarget, SyncMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn ingest(
    archive: &mut Archive,
    blobs: &FileBlobs,
    mailbox: i64,
    uid: Option<u32>,
    origin: &str,
    keywords: &[&str],
) -> Result<MessageId> {
    Ok(archive.ingest_with_keywords(
        &StalwartParser::default(),
        blobs,
        IncomingMessage {
            account: "a",
            mailbox,
            origin,
            epoch: if uid.is_some() { 7 } else { 0 },
            uid,
            seen: false,
            starred: false,
            received_at: 1,
            raw: b"From: peer@example.org\r\nSubject: Synthetic keyword\r\n\r\nBody",
        },
        &keywords.iter().map(|s| (*s).into()).collect::<Vec<_>>(),
    )?)
}

fn item(uid: u32, keywords: &[&str]) -> SyncMessage {
    SyncMessage {
        uid,
        seen: false,
        starred: false,
        size: 100,
        received_at: 1,
        keywords: keywords.iter().map(|s| (*s).into()).collect(),
    }
}

#[test]
fn keyword_atoms_bulk_rollback_and_durable_aba_acknowledgments() -> Result {
    for value in [
        "", "\\Seen", "a b", "a\r\nb", "a)", "a*", "a%", "a]", "a{", "a\"", "é",
    ] {
        assert!(canonical_keyword(value).is_none(), "{value:?}");
    }
    assert!(canonical_keyword(&"a".repeat(256)).is_none());
    assert_eq!(canonical_keyword("$Label1"), Some("$label1".into()));
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let source = archive.mailbox("a", "INBOX")?;
    let destination = archive.mailbox("a", "Other")?;
    let a = ingest(
        &mut archive,
        &blobs,
        source,
        Some(1),
        "one",
        &["$Label1", "$label1"],
    )?;
    let b = ingest(&mut archive, &blobs, source, Some(2), "two", &[])?;
    assert_eq!(archive.message_keywords(a)?, ["$label1"]);
    assert!(
        archive
            .set_keywords(&[a, MessageId(9999)], "work", true)
            .is_err()
    );
    assert!(archive.keyword_operations("a", 0)?.is_empty());
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch("CREATE TRIGGER fail_keyword BEFORE INSERT ON keyword_operations WHEN NEW.uid=2 BEGIN SELECT RAISE(ABORT,'synthetic journal failure'); END;")?;
    assert!(archive.set_keywords(&[a, b], "work", true).is_err());
    assert_eq!(archive.message_keywords(a)?, ["$label1"]);
    assert!(archive.keyword_operations("a", 0)?.is_empty());
    db.execute_batch("DROP TRIGGER fail_keyword;")?;
    archive.set_keywords(&[a, b, a], "WORK", true)?;
    let old = archive.keyword_operations("a", 0)?[0].id;
    archive.set_keywords(&[a], "work", false)?;
    archive.set_keywords(&[a], "work", true)?;
    archive.acknowledge_keyword(old)?;
    assert_eq!(archive.keyword_operations("a", 0)?.len(), 2);
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    let newer = archive
        .keyword_operations("a", old)?
        .last()
        .ok_or("missing new intent")?
        .id;
    assert!(newer > old);
    archive.block_keyword(old, "late stale completion")?;
    assert!(
        archive
            .keyword_operations("a", 0)?
            .iter()
            .all(|op| op.state == "queued")
    );
    archive.queue_copies(&[b], Some(source), MoveTarget::Folder(destination))?;
    assert!(archive.set_keywords(&[a, b], "rollback", true).is_err());
    assert!(!archive.message_keywords(a)?.iter().any(|s| s == "rollback"));
    assert!(archive.set_keywords(&vec![a; 101], "work", true).is_err());
    Ok(())
}

#[test]
fn complete_inventory_keyword_publication_pending_precedence_epoch_and_ingest_atomicity() -> Result
{
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let source = archive.mailbox("a", "INBOX")?;
    let a = ingest(
        &mut archive,
        &blobs,
        source,
        Some(1),
        "one",
        &["old", "remove"],
    )?;
    archive.set_keywords(&[a], "local", true)?;
    archive.set_keywords(&[a], "remove", false)?;
    let pending = archive.keyword_operations("a", 0)?;
    archive.block_keyword(pending[0].id, "unsupported")?;
    archive.begin_sync_inventory()?;
    archive.stage_sync_inventory(7, 2, &[item(1, &["REMOTE", "remove"]), item(2, &["new"])])?;
    assert_eq!(archive.message_keywords(a)?, ["local", "old"]);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch("CREATE TRIGGER fail_keywords BEFORE INSERT ON occurrence_keywords WHEN NEW.keyword='remote' BEGIN SELECT RAISE(ABORT,'synthetic merge failure'); END;")?;
    assert!(archive.finish_sync_inventory(source).is_err());
    assert_eq!(archive.message_keywords(a)?, ["local", "old"]);
    db.execute_batch("DROP TRIGGER fail_keywords;")?;
    archive.finish_sync_inventory(source)?;
    assert_eq!(archive.message_keywords(a)?, ["local", "remote"]);
    let missing = archive.missing_sync_messages(u64::MAX)?;
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].keywords, ["new"]);
    // Invalid keyword metadata never publishes bytes as an authoritative message.
    assert!(ingest(&mut archive, &blobs, source, Some(2), "bad", &["bad atom"]).is_err());
    assert_eq!(archive.total()?, 1);
    for op in pending {
        archive.acknowledge_keyword(op.id)?;
    }
    archive.begin_sync_inventory()?;
    archive.stage_sync_inventory(7, 2, &[item(1, &["server-only"])])?;
    archive.finish_sync_inventory(source)?;
    assert_eq!(archive.message_keywords(a)?, ["server-only"]);
    archive.set_keywords(&[a], "stale", true)?;
    archive.begin_sync_inventory()?;
    archive.stage_sync_inventory(8, 1, &[item(1, &["replacement"])])?;
    archive.finish_sync_inventory(source)?;
    assert!(archive.message_keywords(a)?.is_empty());
    assert_eq!(archive.keyword_operations("a", 0)?[0].state, "blocked");
    Ok(())
}

#[test]
fn local_transfers_labels_live_search_and_v16_migration() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let source = archive.mailbox("a", "INBOX")?;
    let destination = archive.mailbox("a", "Other")?;
    let a = ingest(&mut archive, &blobs, source, None, "one", &["Work"])?;
    archive.queue_copies(&[a], Some(source), MoveTarget::Folder(destination))?;
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute(
        "DELETE FROM occurrence_keywords WHERE mailbox_id=?1",
        [destination],
    )?;
    archive.queue_copies(&[a], Some(source), MoveTarget::Folder(destination))?;
    let count: i64 = db.query_row(
        "SELECT COUNT(*) FROM occurrence_keywords WHERE mailbox_id=?1",
        [destination],
        |r| r.get(0),
    )?;
    assert_eq!(
        count, 0,
        "repeated copy preserves existing destination state"
    );
    archive.queue_moves(&[a], Some(source), MoveTarget::Folder(destination))?;
    assert_eq!(archive.message_keywords(a)?, ["work"]);
    archive.set_keyword_label("a", "WORK", "Work projects")?;
    archive.set_keyword_label("b", "work", "Personal")?;
    let index = SqliteIndex::in_memory()?;
    let query = MailQuery {
        search_matches: true,
        ..Default::default()
    };
    let predicate = parse_query("keyword:WORK AND tag:\"Work projects\"")?;
    archive.prepare_predicate(&index, &predicate)?;
    assert_eq!(archive.page(&query, 10)?.rows[0].id, a);
    let saved = archive.save_search(
        "Tagged",
        "keyword:WORK AND tag:\"Work projects\"",
        &predicate,
    )?;
    archive.set_keyword_label("a", "work", "Renamed")?;
    assert!(
        archive.page(&query, 10)?.rows.is_empty(),
        "labels are live, account-scoped metadata"
    );
    assert!(archive.set_keyword_label("a", "other", "Renamed").is_err());
    assert_eq!(archive.keyword_labels("a")?.len(), 1);
    archive.prepare_predicate(&index, &parse_query("NOT tag:Personal AND keyword:work")?)?;
    assert_eq!(archive.page(&query, 10)?.rows.len(), 1);
    archive.set_keywords(&[a], "work", false)?;
    assert!(archive.page(&query, 10)?.rows.is_empty());
    drop(archive);
    let archive = Archive::open(dir.path())?;
    assert_eq!(archive.saved_search(saved)?.predicate, predicate);
    drop(archive);
    db.execute_batch(include_str!("fixtures/remove_sent_copy.sql"))?;
    db.execute_batch("DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules; DROP TABLE occurrence_keywords; DROP TABLE keyword_operations; DROP TABLE keyword_labels; PRAGMA user_version=16;")?;
    let archive = Archive::open(dir.path())?;
    assert_eq!(archive.total()?, 1);
    assert!(archive.message_keywords(a)?.is_empty());
    assert!(archive.keyword_operations("a", 0)?.is_empty());
    assert_eq!(
        db.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
        20
    );
    Ok(())
}
