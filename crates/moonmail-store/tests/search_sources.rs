use {
    moonmail_core::{IndexChange, MailPredicate, MailQuery, MailRepository, SearchIndex},
    moonmail_mime::StalwartParser,
    moonmail_search::{SqliteIndex, parse_query},
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn nested_sources_union_then_filter_live_mail_and_parent_edits_survive_restart() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("me@example.org", "INBOX")?;
    let mut index = SqliteIndex::in_memory()?;
    let mut ids = Vec::new();
    for (n, (sender, seen)) in [
        ("alice", false),
        ("bob", false),
        ("alice", true),
        ("carol", false),
    ]
    .into_iter()
    .enumerate()
    {
        let raw = format!("From: {sender}@example.org\r\nSubject: Review\r\n\r\nSynthetic");
        ids.push(archive.ingest(
            &StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            IncomingMessage {
                account: "me@example.org",
                mailbox: inbox,
                origin: &n.to_string(),
                epoch: 0,
                uid: None,
                seen,
                starred: false,
                received_at: n as i64,
                raw: raw.as_bytes(),
            },
        )?);
    }
    for work in archive.index_work(100)? {
        index.apply(&[IndexChange::Upsert(work.id, &work.document)])?;
    }
    let alice = archive.save_search("Alice", "from:alice", &parse_query("from:alice")?)?;
    let bob = archive.save_search("Bob", "from:bob", &parse_query("from:bob")?)?;
    let unread = archive.save_search_from(
        "Unread peers",
        "is:unread",
        &parse_query("is:unread")?,
        &[alice, bob, alice],
    )?;
    let child = archive.save_search_from(
        "Review peers",
        "subject:review",
        &parse_query("subject:review")?,
        &[unread],
    )?;
    assert_eq!(archive.saved_search(unread)?.sources, vec![alice, bob]);
    let query = MailQuery {
        search_matches: true,
        ..Default::default()
    };
    archive.prepare_predicate(&index, &archive.resolve_search(child)?.1)?;
    assert_eq!(
        archive
            .page(&query, 10)?
            .rows
            .iter()
            .map(|row| row.id)
            .collect::<Vec<_>>(),
        vec![ids[1], ids[0]]
    );
    archive.set_flag(ids[0], "seen", true)?;
    assert_eq!(archive.page(&query, 10)?.rows[0].id, ids[1]);
    assert_eq!(archive.page(&query, 10)?.rows.len(), 1);
    archive.save_search("Bob", "from:carol", &parse_query("from:carol")?)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    archive.prepare_predicate(&index, &archive.resolve_search(child)?.1)?;
    assert_eq!(archive.page(&query, 10)?.rows[0].id, ids[3]);
    assert_eq!(archive.page(&query, 10)?.rows.len(), 1);
    archive.prepare_predicate(
        &index,
        &parse_query("subject:review is:unread (from:alice OR from:carol)")?,
    )?;
    assert_eq!(archive.page(&query, 10)?.rows[0].id, ids[3]);
    assert!(archive.delete_search(alice).is_err());
    archive.delete_search(child)?;
    archive.delete_search(unread)?;
    archive.delete_search(alice)?;
    Ok(())
}

#[test]
fn cyclic_missing_or_oversized_source_edits_rollback_the_entire_change() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let predicate = parse_query("is:unread")?;
    let parent = archive.save_search("Parent", "is:unread", &predicate)?;
    let child = archive.save_search_from("Child", "is:unread", &predicate, &[parent])?;
    let grandchild = archive.save_search_from("Grandchild", "is:unread", &predicate, &[child])?;
    for sources in [vec![parent], vec![grandchild], vec![9999]] {
        assert!(
            archive
                .save_search_from("Parent", "is:starred", &MailPredicate::Starred, &sources)
                .is_err()
        );
        let saved = archive.saved_search(parent)?;
        assert_eq!(saved.source, "is:unread");
        assert!(saved.sources.is_empty());
    }
    let large = MailPredicate::All(vec![MailPredicate::Unread; 60]);
    assert!(
        archive.save_search("Parent", "large", &large).is_err(),
        "descendant expansion exceeds the shared budget"
    );
    assert_eq!(archive.saved_search(parent)?.predicate, predicate);
    assert!(
        archive
            .save_search_from("New invalid", "is:unread", &predicate, &[9999])
            .is_err()
    );
    assert_eq!(archive.saved_searches()?.len(), 3);
    drop(archive);
    let archive = Archive::open(dir.path())?;
    assert!(archive.resolve_search(grandchild).is_ok());
    assert_eq!(archive.saved_search(parent)?.predicate, predicate);
    Ok(())
}

#[test]
fn version_twelve_saved_queries_gain_empty_sources_without_changing_their_ast() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let predicate = parse_query("in:INBOX is:unread")?;
    let id = archive.save_search("Inbox", "in:INBOX is:unread", &predicate)?;
    drop(archive);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(concat!(
        include_str!("fixtures/remove_sent_copy.sql"),
        "DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules;"
    ))?;
    db.execute_batch("DROP TABLE occurrence_keywords; DROP TABLE keyword_labels; DROP TABLE keyword_operations; ALTER TABLE move_operations DROP COLUMN kind; DROP TABLE folder_operations; DROP TABLE thread_pending; DROP TABLE thread_edges; DROP TABLE thread_members; DROP TABLE thread_nodes; DROP TABLE search_sources; PRAGMA user_version=12;")?;
    drop(db);
    let archive = Archive::open(dir.path())?;
    let (saved, resolved) = archive.resolve_search(id)?;
    assert!(saved.sources.is_empty());
    assert_eq!(resolved, predicate);
    Ok(())
}
