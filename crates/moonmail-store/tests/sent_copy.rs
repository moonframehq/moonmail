use {
    moonmail_core::BlobStore,
    moonmail_store::{Archive, Draft, FileBlobs},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn version_nineteen_migration_preserves_outbox_without_inventing_append_intent() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let draft = Draft {
        id: "legacy".into(),
        account: "me@example.org".into(),
        ..Default::default()
    };
    let blob = FileBlobs::new(dir.path()).put(b"Synthetic original")?;
    let id = archive.queue(&draft, &draft.account, &blob, &["peer@example.org".into()])?;
    drop(archive);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(include_str!("fixtures/remove_sent_copy.sql"))?;
    db.execute_batch("PRAGMA user_version=19;")?;
    let archive = Archive::open(dir.path())?;
    let item = archive.outbox_item(id)?;
    assert_eq!(item.state, "queued");
    assert_eq!(item.blob, blob);
    assert_eq!(item.sent_copy_state, "server");
    assert!(archive.pending_sent_copies(&draft.account)?.is_empty());
    archive.prepare_sent_copy(id, true)?;
    assert!(!archive.claim_sent_copy(id, "Sent")?);
    assert!(archive.claim_send(id)?);
    archive.send_result(id, "sent", "Accepted")?;
    assert_eq!(archive.pending_sent_copies(&draft.account)?, vec![id]);
    assert!(archive.claim_sent_copy(id, "Sent")?);
    assert!(!archive.claim_sent_copy(id, "Other")?);
    assert!(archive.prepare_sent_copy(id, false).is_err());
    Ok(())
}
