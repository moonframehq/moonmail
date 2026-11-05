use {
    moonmail_core::{MailQuery, MailRepository},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage, SyncMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn item(uid: u32) -> SyncMessage {
    SyncMessage {
        keywords: Vec::new(),
        uid,
        seen: true,
        starred: false,
        size: 120,
        received_at: 1_700_000_000 + i64::from(uid),
    }
}

#[test]
fn partial_inventory_is_invisible_and_complete_merge_is_atomic_with_local_flag_precedence() -> Result
{
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let mailbox = archive.mailbox("me@example.org", "INBOX")?;
    let mut ids = Vec::new();
    for uid in [1, 2, 3] {
        ids.push(archive.ingest(
            &StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            IncomingMessage {
                account: "me@example.org",
                mailbox,
                origin: &uid.to_string(),
                epoch: 7,
                uid: Some(uid),
                seen: false,
                starred: false,
                received_at: 0,
                raw: b"From: peer@example.org\r\nSubject: Inventory\r\n\r\nBody",
            },
        )?);
    }
    archive.set_flag(ids[0], "flagged", true)?;
    archive.begin_sync_inventory()?;
    archive.stage_sync_inventory(7, 2, &[item(1)])?;
    assert_eq!(archive.page(&MailQuery::default(), 10)?.rows.len(), 3);
    assert!(archive.missing_sync_messages(u64::MAX)?.is_empty());
    assert!(archive.stage_sync_inventory(8, 2, &[]).is_err());
    assert!(archive.stage_sync_inventory(7, 2, &[item(3)]).is_err());
    // Inject failure after the delete step, verifying the merge rolls back fully.
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch("CREATE TRIGGER fail_merge BEFORE UPDATE ON occurrences BEGIN SELECT RAISE(ABORT,'synthetic merge failure'); END;")?;
    assert!(archive.finish_sync_inventory(mailbox).is_err());
    assert_eq!(archive.page(&MailQuery::default(), 10)?.rows.len(), 3);
    db.execute_batch("DROP TRIGGER fail_merge;")?;
    assert_eq!(archive.finish_sync_inventory(mailbox)?, 7);
    let rows = archive.page(&MailQuery::default(), 10)?.rows;
    assert_eq!(rows.len(), 2); // uid=2 expunged, uid=3 beyond fixed upper retained
    let retained = rows
        .iter()
        .find(|row| row.id == ids[0])
        .ok_or("missing retained")?;
    assert!(!retained.unread && retained.starred);
    assert!(rows.iter().any(|row| row.id == ids[2]));
    assert!(archive.finish_sync_inventory(mailbox).is_err());
    assert_eq!(archive.total()?, 3); // immutable archive survives expunges
    archive.begin_sync_inventory()?;
    archive.stage_sync_inventory(8, 0, &[])?;
    archive.finish_sync_inventory(mailbox)?;
    assert!(archive.page(&MailQuery::default(), 10)?.rows.is_empty());
    Ok(())
}

#[test]
fn staged_inventory_pages_thousands_of_missing_uids_and_rejects_duplicate_batches() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let mailbox = archive.mailbox("me@example.org", "INBOX")?;
    archive.begin_sync_inventory()?;
    for start in [1, 1001, 2001] {
        let messages: Vec<_> = (start..=(start + 999).min(2505)).map(item).collect();
        archive.stage_sync_inventory(7, 2505, &messages)?;
    }
    assert!(
        archive
            .stage_sync_inventory(7, 2505, &[item(1), item(2)])
            .is_err()
    );
    archive.finish_sync_inventory(mailbox)?;
    let mut before = u64::MAX;
    let mut found = Vec::new();
    loop {
        let page = archive.missing_sync_messages(before)?;
        assert!(page.len() <= 100);
        if page.is_empty() {
            break;
        }
        for message in page {
            assert!(u64::from(message.uid) < before);
            before = u64::from(message.uid);
            found.push(message.uid);
        }
    }
    assert_eq!(found, (1..=2505).rev().collect::<Vec<_>>());
    drop(archive);
    let archive = Archive::open(dir.path())?;
    archive.begin_sync_inventory()?;
    assert!(archive.missing_sync_messages(u64::MAX)?.is_empty());
    Ok(())
}
