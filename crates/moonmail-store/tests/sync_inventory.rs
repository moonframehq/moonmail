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

#[test]
fn checkpoint_requires_durable_downloads_and_a_current_generation() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let mailbox = archive.mailbox("fixture", "INBOX")?;
    archive.begin_sync_inventory()?;
    archive.stage_sync_inventory(7, 1, &[item(1)])?;
    archive.finish_sync_inventory(mailbox)?;
    archive.publish_sync_checkpoint(mailbox, u64::MAX)?;
    assert_eq!(archive.sync_checkpoint(mailbox)?, None);
    archive.ingest(
        &StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        IncomingMessage {
            mailbox,
            epoch: 7,
            uid: Some(1),
            account: "fixture",
            origin: "fixture:1",
            received_at: 1,
            seen: false,
            starred: false,
            raw: b"Subject: Test\r\n\r\nBody",
        },
    )?;
    archive.publish_sync_checkpoint(mailbox, u64::MAX)?;
    let mut second = Archive::open(dir.path())?;
    assert_eq!(
        second
            .sync_checkpoint(mailbox)?
            .ok_or("missing checkpoint")?
            .highest_modseq,
        u64::MAX
    );
    second.begin_sync_inventory()?;
    second.stage_sync_inventory(8, 0, &[])?;
    second.finish_sync_inventory(mailbox)?;
    archive.publish_sync_checkpoint(mailbox, u64::MAX)?;
    assert_eq!(archive.sync_checkpoint(mailbox)?, None);
    Ok(())
}

#[test]
fn vanished_ranges_commit_only_after_complete_delta_and_preserve_survivors() -> Result {
    use moonmail_store::InventoryKind;
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let mailbox = archive.mailbox("fixture", "INBOX")?;
    archive.begin_sync_inventory()?;
    archive.stage_sync_inventory(7, 5, &[item(1), item(3), item(5)])?;
    archive.finish_sync_inventory(mailbox)?;
    for uid in [1, 3, 5] {
        archive.ingest_staged(
            &StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            IncomingMessage {
                mailbox,
                epoch: 7,
                uid: Some(uid),
                account: "fixture",
                origin: &uid.to_string(),
                seen: false,
                starred: false,
                received_at: 1,
                raw: b"Subject: Test\r\n\r\nBody",
            },
            &[],
        )?;
    }
    archive.publish_sync_checkpoint(mailbox, 6)?;
    let base = archive
        .begin_mailbox_inventory(mailbox)?
        .ok_or("missing checkpoint")?;
    archive.stage_sync_inventory(7, 5, &[])?;
    assert!(archive.stage_sync_vanished(8, 5, 1, 4).is_err());
    archive.stage_sync_vanished(7, 5, 1, 4)?;
    assert_eq!(archive.page(&Default::default(), 10)?.rows.len(), 3);
    archive.merge_sync_inventory(mailbox, InventoryKind::Vanished(base))?;
    archive.publish_sync_checkpoint(mailbox, 7)?;
    assert_eq!(archive.page(&Default::default(), 10)?.rows.len(), 1);
    assert_eq!(archive.total()?, 3);
    assert_eq!(
        archive
            .sync_checkpoint(mailbox)?
            .ok_or("missing delta checkpoint")?
            .highest_modseq,
        7
    );
    Ok(())
}
