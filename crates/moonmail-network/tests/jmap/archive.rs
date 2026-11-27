use super::*;

#[test]
fn stored_memberships_inventory_and_invalidation_survive_reopen() -> Result {
    use {
        moonmail_mime::StalwartParser,
        moonmail_store::{IncomingMessage, JmapFolder},
    };
    let root = tempfile::tempdir()?;
    let mut db = Archive::open(root.path())?;
    db.bind_jmap(ACCOUNT, "account")?;
    db.jmap_folders(
        ACCOUNT,
        &[JmapFolder {
            remote: "in".into(),
            path: "Inbox".into(),
            role: Some(MailboxRole::Inbox),
        }],
        "f",
    )?;
    let folder = db.role_mailbox(ACCOUNT, MailboxRole::Inbox)?;
    assert!(db.is_jmap_folder(folder)?);
    let uid = db.jmap_uid(ACCOUNT, "m")?;
    assert_eq!(uid, db.jmap_uid(ACCOUNT, "m")?);
    let id = db.ingest(
        &StalwartParser::default(),
        &FileBlobs::new(root.path()),
        IncomingMessage {
            account: ACCOUNT,
            mailbox: folder,
            origin: "jmap:m",
            epoch: 0,
            uid: Some(uid),
            seen: false,
            starred: false,
            received_at: 1,
            raw: MIME,
        },
    )?;
    db.observe_jmap(id, uid, &[folder], true, true, &["Label".into()])?;
    db.begin_jmap_inventory()?;
    db.stage_jmap_id("m")?;
    db.checkpoint_jmap(ACCOUNT, "s", None, &[], true)?;
    drop(db);
    let db = Archive::open(root.path())?;
    assert_eq!(db.jmap_message(ACCOUNT, "m")?, Some(id));
    assert_eq!(db.message_keywords(id)?, ["label"]);
    db.invalidate_jmap(ACCOUNT)?;
    assert_eq!(db.jmap_states(ACCOUNT)?, (None, None));
    Ok(())
}

#[test]
fn failed_archive_writes_do_not_publish_new_handles_or_advance_checkpoints() -> Result {
    let root = tempfile::tempdir()?;
    let (mut db, _, _, _) = journal_fixture(root.path())?;
    let sql = rusqlite::Connection::open(root.path().join("archive.sqlite3"))?;
    sql.execute_batch(
        "CREATE TRIGGER fail_handle BEFORE INSERT ON jmap_emails
        WHEN NEW.remote='fail' BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;",
    )?;
    assert!(db.jmap_email_handles(ACCOUNT, &["new", "fail"]).is_err());
    let count: u32 = sql.query_row(
        "SELECT count(*) FROM jmap_emails WHERE remote IN ('new','fail')",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(count, 0);
    assert!(db.jmap_remote(ACCOUNT, 123456).is_err());
    assert!(db.jmap_states("unbound").is_err());
    assert!(db.stage_jmap_ids(&["new"]).is_err());
    sql.execute_batch(
        "CREATE TRIGGER fail_checkpoint BEFORE UPDATE ON jmap_accounts
        BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;
        CREATE TRIGGER fail_binding BEFORE INSERT ON jmap_accounts
        BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;",
    )?;
    assert!(db.bind_jmap("new-account", "id").is_err());
    assert!(db.invalidate_jmap(ACCOUNT).is_err());
    assert!(
        db.checkpoint_jmap(ACCOUNT, "later", None, &["m".into()], false)
            .is_err()
    );
    assert_eq!(db.jmap_states(ACCOUNT)?.1.as_deref(), Some("s"));
    let count: u32 = sql.query_row("SELECT count(*) FROM occurrences", [], |r| r.get(0))?;
    assert_eq!(count, 1);
    sql.execute_batch(
        "ALTER TABLE jmap_accounts RENAME TO unavailable_accounts;
        ALTER TABLE jmap_emails RENAME TO unavailable_handles;
        ALTER TABLE jmap_folders RENAME TO unavailable_folders;",
    )?;
    drop(db);
    let mut db = Archive::open(root.path())?;
    assert!(db.is_jmap_folder(1).is_err());
    assert!(db.bound_jmap_states(ACCOUNT, "account").is_err());
    assert!(db.jmap_email_handles(ACCOUNT, &["m"]).is_err());
    assert!(db.jmap_flags(ACCOUNT, 0, 1).is_err());
    assert!(db.jmap_deletions(ACCOUNT, 1).is_err());
    sql.execute_batch("CREATE VIEW jmap_emails AS SELECT * FROM unavailable_handles")?;
    assert!(db.jmap_email_handles(ACCOUNT, &["missing"]).is_err());
    Ok(())
}
