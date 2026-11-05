use {
    moonmail_core::{MailPredicate, MessageId},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage, KeywordRule, MoveTarget},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn rule(mailbox: i64, name: &str, keyword: &str) -> KeywordRule {
    KeywordRule {
        id: 0,
        revision: 0,
        mailbox,
        name: name.into(),
        source: String::new(),
        predicate: MailPredicate::Account("a".into()),
        keyword: keyword.into(),
        value: true,
        enabled: true,
    }
}
fn ingest(
    archive: &mut Archive,
    blobs: &FileBlobs,
    mailbox: i64,
    origin: &str,
    uid: Option<u32>,
) -> Result<MessageId> {
    Ok(archive.ingest(
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
            raw: b"From: peer@example.org\r\nSubject: Synthetic entry\r\n\r\nBody",
        },
    )?)
}

#[test]
fn entries_apply_in_order_without_replay_on_reingest_restart_reindex_or_existing_copy() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("a", "INBOX")?;
    let other = archive.mailbox("a", "Other")?;
    let old = ingest(&mut archive, &blobs, inbox, "old", None)?;
    archive.save_keyword_rule(&rule(inbox, "First", "first"))?;
    let mut second = rule(inbox, "Second", "second");
    second.source = "keyword:first".into();
    second.predicate = MailPredicate::Keyword("first".into());
    archive.save_keyword_rule(&second)?;
    assert!(
        archive.message_keywords(old)?.is_empty(),
        "creating rules has no retrospective effect"
    );
    let fresh = ingest(&mut archive, &blobs, inbox, "fresh", None)?;
    assert_eq!(archive.message_keywords(fresh)?, ["first", "second"]);
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 2);
    archive.set_keywords(&[fresh], "second", false)?;
    ingest(&mut archive, &blobs, inbox, "fresh", None)?;
    assert_eq!(
        archive.message_keywords(fresh)?,
        ["first"],
        "duplicate local imports preserve user changes without replaying rules"
    );
    archive.reset_index()?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 2);
    archive.save_keyword_rule(&rule(other, "Copy entry", "copied"))?;
    archive.queue_copies(&[fresh], Some(inbox), MoveTarget::Folder(other))?;
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 3);
    assert_eq!(archive.message_keywords(fresh)?, ["copied", "first"]);
    archive.queue_copies(&[fresh], Some(inbox), MoveTarget::Folder(other))?;
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 3);
    archive.queue_moves(&[fresh], Some(inbox), MoveTarget::Folder(other))?;
    assert_eq!(
        archive.keyword_rule_runs(None)?.len(),
        3,
        "moving into existing membership is not entry"
    );
    archive.queue_moves(&[fresh], Some(other), MoveTarget::Folder(inbox))?;
    assert_eq!(
        archive.keyword_rule_runs(None)?.len(),
        5,
        "genuine exit/reentry creates a new event"
    );
    assert_eq!(
        archive.message_keywords(fresh)?,
        ["copied", "first", "second"]
    );
    Ok(())
}

#[test]
fn rule_action_and_entry_rollback_together_and_remote_intent_survives_restart() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("a", "INBOX")?;
    archive.save_keyword_rule(&rule(inbox, "Remote action", "work"))?;
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch("CREATE TRIGGER fail_rule BEFORE INSERT ON keyword_rule_runs BEGIN SELECT RAISE(ABORT,'synthetic rule-log failure'); END;")?;
    assert!(ingest(&mut archive, &blobs, inbox, "remote", Some(1)).is_err());
    assert_eq!(archive.total()?, 0);
    assert!(archive.keyword_operations("a", 0)?.is_empty());
    db.execute_batch("DROP TRIGGER fail_rule;")?;
    let id = ingest(&mut archive, &blobs, inbox, "remote", Some(1))?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert_eq!(archive.message_keywords(id)?, ["work"]);
    assert_eq!(archive.keyword_operations("a", 0)?.len(), 1);
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 1);
    ingest(&mut archive, &blobs, inbox, "remote", Some(1))?;
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 1);
    archive.reconcile(inbox, 8, &[])?;
    ingest(&mut archive, &blobs, inbox, "new-epoch", Some(1))?;
    assert_eq!(archive.keyword_rule_runs(None)?.len(), 2);
    Ok(())
}

#[test]
fn disabled_manual_rules_revision_guards_blocked_transfers_and_history_paging() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("a", "INBOX")?;
    let other = archive.mailbox("a", "Other")?;
    let mut draft = rule(inbox, "Manual", "work");
    draft.enabled = false;
    let id = archive.save_keyword_rule(&draft)?;
    let mut saved = archive.keyword_rules()?.remove(0);
    let message = ingest(&mut archive, &blobs, inbox, "one", Some(1))?;
    assert!(archive.message_keywords(message)?.is_empty());
    assert!(
        archive
            .apply_keyword_rule(id, saved.revision, &[message, MessageId(9999)])
            .is_err()
    );
    assert!(archive.message_keywords(message)?.is_empty());
    assert!(archive.keyword_rule_runs(None)?.is_empty());
    assert_eq!(
        archive.apply_keyword_rule(id, saved.revision, &[message])?,
        1
    );
    archive.queue_copies(&[message], Some(inbox), MoveTarget::Folder(other))?;
    assert_eq!(
        archive.apply_keyword_rule(id, saved.revision, &[message])?,
        0
    );
    assert_eq!(archive.keyword_rule_runs(None)?[0].state, "blocked");
    archive.cancel_move(archive.move_operations()?[0].id)?;
    for _ in 0..101 {
        archive.apply_keyword_rule(id, saved.revision, &[message])?;
    }
    let first = archive.keyword_rule_runs(None)?;
    assert_eq!(first.len(), 100);
    let next = archive.keyword_rule_runs(first.last().map(|r| r.id))?;
    assert_eq!(next.len(), 3);
    assert!(first.iter().all(|a| next.iter().all(|b| a.id != b.id)));
    saved.enabled = true;
    archive.save_keyword_rule(&saved)?;
    assert!(archive.save_keyword_rule(&saved).is_err());
    assert!(
        archive
            .apply_keyword_rule(id, saved.revision, &[message])
            .is_err()
    );
    assert!(archive.delete_keyword_rule(id, saved.revision).is_err());
    let current = archive.keyword_rules()?.remove(0);
    archive.delete_keyword_rule(id, current.revision)?;
    assert_eq!(
        archive.keyword_rule_runs(None)?.len(),
        100,
        "history survives definition deletion"
    );
    Ok(())
}

#[test]
fn rule_validation_and_v17_upgrade_preserve_keywords_without_synthetic_entry_events() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("a", "INBOX")?;
    let id = ingest(&mut archive, &blobs, inbox, "old", None)?;
    archive.set_keywords(&[id], "kept", true)?;
    let mut bad = rule(inbox, "Bad", "work");
    bad.predicate = moonmail_search::parse_query("subject:hello")?;
    assert!(archive.save_keyword_rule(&bad).is_err());
    bad.predicate = MailPredicate::Unread;
    bad.keyword = "bad atom".into();
    assert!(archive.save_keyword_rule(&bad).is_err());
    bad.keyword = "work".into();
    bad.mailbox = 99999;
    assert!(archive.save_keyword_rule(&bad).is_err());
    assert!(archive.keyword_rules()?.is_empty());
    drop(archive);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(
        concat!(include_str!("fixtures/remove_sent_copy.sql"), "DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules; PRAGMA user_version=17;"),
    )?;
    let archive = Archive::open(dir.path())?;
    assert_eq!(archive.message_keywords(id)?, ["kept"]);
    assert!(archive.keyword_rules()?.is_empty());
    assert!(archive.keyword_rule_runs(None)?.is_empty());
    Ok(())
}
