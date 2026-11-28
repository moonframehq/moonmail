use {moonmail_store::Archive, rusqlite::Connection};

#[test]
fn v30_upgrade_preserves_queued_and_uncertain_imap_deletions()
-> Result<(), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    drop(Archive::open(dir.path())?);
    let sql = Connection::open(dir.path().join("archive.sqlite3"))?;
    sql.execute_batch("DROP TRIGGER draft_cleanup; DROP TRIGGER draft_account_change; DROP TRIGGER jmap_deletion_arrival;
        DROP TABLE jmap_deletion_revisions; DROP TABLE expunge_operations;")?;
    // Use the original schema and triggers to exercise migration from the shipped archive.
    let original = include_str!("../src/remote_drafts.sql");
    let start = original
        .find("CREATE TABLE expunge_operations")
        .ok_or("missing deletion schema")?;
    let end = original
        .find("CREATE TABLE deletion_inventory")
        .ok_or("missing inventory schema")?;
    sql.execute_batch(&original[start..end])?;
    sql.execute_batch(
        "INSERT INTO expunge_operations(id,account,mailbox,epoch,uid,state) VALUES
        (12,'fixture','Deleted',42,1,'queued'),(13,'fixture','Deleted',42,2,'unknown');
        PRAGMA user_version=30;",
    )?;
    drop(sql);
    let db = Archive::open(dir.path())?;
    let pending = db.remote_operations(Some("fixture"))?;
    assert_eq!(pending.len(), 1);
    assert_eq!(
        (pending[0].id, pending[0].epoch, pending[0].uid),
        (12, 42, 1)
    );
    let history = db.remote_operations(None)?;
    assert!(
        history
            .iter()
            .any(|op| op.id == 13 && op.state == "unknown")
    );
    assert!(db.jmap_deletions("fixture", 100)?.is_empty());
    Ok(())
}
