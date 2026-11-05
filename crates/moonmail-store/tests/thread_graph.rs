use {
    moonmail_core::{MailQuery, MailRepository, MessageId, RelatedQuery, Relationship, SortField},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
    rusqlite::Connection,
    std::collections::BTreeSet,
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn entire_thread_flags_cover_all_pages_and_rollback_if_any_member_is_moving() -> Result {
    let mut f = Fixture::new()?;
    let mut ids = Vec::new();
    for n in 0..237 {
        ids.push(f.add(
            "a",
            &format!("{n}@example.org"),
            "References: <root@example.org>\r\n",
        )?);
    }
    let other = f.add(
        "b",
        "same-root@example.org",
        "References: <root@example.org>\r\n",
    )?;
    assert_eq!(f.archive.set_thread_seen(ids[0], false)?, 237);
    assert_eq!(f.archive.pending_flags("a")?.len(), 237);
    assert!(!f.archive.detail(other)?.summary.unread);
    let destination = f.archive.mailbox("a", "Archive")?;
    f.archive.queue_moves(
        &[ids[236]],
        None,
        moonmail_store::MoveTarget::Folder(destination),
    )?;
    assert!(f.archive.set_thread_seen(ids[0], true).is_err());
    for id in &ids {
        assert!(f.archive.detail(*id)?.summary.unread);
    }
    assert!(f.archive.pending_flags("a")?.iter().all(|flag| !flag.value));
    f.archive.cancel_move(f.archive.move_operations()?[0].id)?;
    let db = f.db()?;
    db.execute_batch("CREATE TRIGGER fail_flag BEFORE UPDATE OF seen ON occurrences WHEN OLD.uid=200 BEGIN SELECT RAISE(ABORT,'synthetic flag failure'); END;")?;
    assert!(f.archive.set_thread_seen(ids[0], true).is_err());
    assert!(f.archive.detail(ids[0])?.summary.unread);
    assert!(f.archive.pending_flags("a")?.iter().all(|flag| !flag.value));
    db.execute_batch("DROP TRIGGER fail_flag;")?;
    assert_eq!(f.archive.set_thread_seen(ids[0], true)?, 237);
    f.archive = Archive::open(f.dir.path())?;
    assert!(f.archive.pending_flags("a")?.iter().all(|flag| flag.value));
    assert!(!f.archive.detail(ids[236])?.summary.unread);
    Ok(())
}

struct Fixture {
    dir: tempfile::TempDir,
    archive: Archive,
    blobs: FileBlobs,
    sequence: u32,
}

impl Fixture {
    fn new() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let archive = Archive::open(dir.path())?;
        let blobs = FileBlobs::new(dir.path());
        Ok(Self {
            dir,
            archive,
            blobs,
            sequence: 0,
        })
    }

    fn add(&mut self, account: &str, id: &str, headers: &str) -> Result<MessageId> {
        self.sequence += 1;
        let mailbox = self.archive.mailbox(account, "INBOX")?;
        let raw = format!(
            "From: sender@example.org\r\nTo: reader@example.org\r\nMessage-ID: <{id}>\r\n{headers}\r\nSynthetic"
        );
        Ok(self.archive.ingest(
            &StalwartParser::default(),
            &self.blobs,
            IncomingMessage {
                account,
                mailbox,
                origin: &self.sequence.to_string(),
                epoch: 1,
                uid: Some(self.sequence),
                seen: true,
                starred: false,
                received_at: i64::from(self.sequence),
                raw: raw.as_bytes(),
            },
        )?)
    }

    fn members(&self, id: MessageId) -> Result<BTreeSet<MessageId>> {
        Ok(self
            .archive
            .thread(id, 200)?
            .into_iter()
            .map(|row| row.id)
            .collect())
    }

    fn db(&self) -> Result<Connection> {
        Ok(Connection::open(self.dir.path().join("archive.sqlite3"))?)
    }
}

#[test]
fn late_parents_bridge_missing_ancestors_and_all_views_agree() -> Result {
    let mut f = Fixture::new()?;
    let leaf = f.add(
        "a",
        "leaf@example.org",
        "In-Reply-To: <middle@example.org>\r\n",
    )?;
    let sibling = f.add(
        "a",
        "sibling@example.org",
        "References: <root@example.org>\r\n",
    )?;
    assert_eq!(f.members(leaf)?, BTreeSet::from([leaf]));
    let middle = f.add(
        "a",
        "middle@example.org",
        "References: <root@example.org>\r\n",
    )?;
    let root = f.add("a", "root@example.org", "")?;
    let other = f.add("b", "root@example.org", "")?;
    let expected = BTreeSet::from([root, middle, leaf, sibling]);
    assert_eq!(f.members(leaf)?, expected);
    assert_eq!(f.members(other)?, BTreeSet::from([other]));
    let groups = f.archive.page(
        &MailQuery {
            threaded: true,
            ..Default::default()
        },
        10,
    )?;
    assert_eq!(groups.rows.len(), 2);
    assert_eq!(
        groups
            .rows
            .iter()
            .find(|row| row.id == root)
            .ok_or("missing root")?
            .thread_count,
        4
    );
    let expanded = f.archive.page(
        &MailQuery {
            thread: Some(leaf),
            ..Default::default()
        },
        10,
    )?;
    assert_eq!(
        expanded
            .rows
            .into_iter()
            .map(|row| row.id)
            .collect::<BTreeSet<_>>(),
        expected
    );
    let related = f.archive.related_page(
        &RelatedQuery {
            anchor: middle,
            relationship: Relationship::Thread,
            sort: SortField::Date,
            ascending: true,
            before: None,
        },
        10,
    )?;
    assert_eq!(
        related
            .rows
            .into_iter()
            .map(|row| row.id)
            .chain([related.anchor.id])
            .collect::<BTreeSet<_>>(),
        expected
    );
    f.archive = Archive::open(f.dir.path())?;
    assert_eq!(f.members(root)?, expected);
    Ok(())
}

#[test]
fn duplicate_ids_split_old_components_but_identical_copies_and_explicit_edges_remain() -> Result {
    let mut f = Fixture::new()?;
    let root = f.add("a", "root@example.org", "")?;
    let copy = f.add("a", "root@example.org", "")?;
    let reply = f.add(
        "a",
        "reply@example.org",
        "References: <root@example.org>\r\n",
    )?;
    assert_eq!(f.members(root)?, BTreeSet::from([root, copy, reply]));
    let conflict = f.add("a", "root@example.org", "Subject: Conflicting bytes\r\n")?;
    for id in [root, copy, reply, conflict] {
        assert_eq!(f.members(id)?, BTreeSet::from([id]));
    }
    // Explicit ancestry still joins replies to the missing/ambiguous RFC node.
    let sibling = f.add(
        "a",
        "sibling@example.org",
        "References: <root@example.org>\r\n",
    )?;
    assert_eq!(f.members(reply)?, BTreeSet::from([reply, sibling]));
    f.archive = Archive::open(f.dir.path())?;
    assert_eq!(f.members(root)?, BTreeSet::from([root]));
    Ok(())
}

#[test]
fn cycles_terminate_and_long_chains_keep_original_headers_with_a_limit_notice() -> Result {
    let mut f = Fixture::new()?;
    let a = f.add("a", "a@example.org", "References: <b@example.org>\r\n")?;
    let b = f.add("a", "b@example.org", "References: <a@example.org>\r\n")?;
    let self_ref = f.add(
        "a",
        "self@example.org",
        "References: <self@example.org>\r\n",
    )?;
    assert_eq!(f.members(a)?, BTreeSet::from([a, b]));
    assert_eq!(f.members(self_ref)?, BTreeSet::from([self_ref]));
    // Force splitting a cyclic component after removing an own-ID edge.
    let duplicate = f.add("a", "b@example.org", "Subject: Different\r\n")?;
    assert_eq!(f.members(duplicate)?, BTreeSet::from([duplicate]));
    assert_eq!(f.members(a)?, BTreeSet::from([a, b]));
    let headers = (0..300)
        .map(|n| format!("<chain-{n}@example.org>"))
        .collect::<Vec<_>>()
        .join(" ");
    let long = f.add(
        "a",
        "long@example.org",
        &format!("References: {headers}\r\n"),
    )?;
    assert_eq!(f.archive.limited_thread_count()?, 1);
    let retained = f
        .archive
        .reply_headers(long, &StalwartParser::default(), &f.blobs)?;
    assert_eq!(retained.references.len(), 300);
    let first = f.add("a", "chain-0@example.org", "")?;
    let last = f.add("a", "chain-299@example.org", "")?;
    assert_eq!(f.members(long)?, BTreeSet::from([long, first, last]));
    Ok(())
}

#[test]
fn ingest_and_lazy_header_recovery_roll_back_graph_and_metadata_together() -> Result {
    let mut f = Fixture::new()?;
    let root = f.add("a", "root@example.org", "")?;
    let reply = f.add(
        "a",
        "reply@example.org",
        "References: <root@example.org>\r\n",
    )?;
    let db = f.db()?;
    db.execute_batch("CREATE TRIGGER fail_graph BEFORE UPDATE OF component ON thread_nodes BEGIN SELECT RAISE(ABORT,'synthetic graph failure'); END;")?;
    assert!(
        f.add(
            "a",
            "later@example.org",
            "References: <root@example.org>\r\n"
        )
        .is_err()
    );
    assert_eq!(f.members(root)?, BTreeSet::from([root, reply]));
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM messages", [], |row| row
            .get::<_, i64>(0))?,
        2
    );
    db.execute_batch("DROP TRIGGER fail_graph;
        UPDATE messages SET references_json=NULL WHERE rfc_id='reply@example.org';
        CREATE TRIGGER fail_graph BEFORE DELETE ON thread_edges BEGIN SELECT RAISE(ABORT,'synthetic edge failure'); END;")?;
    assert!(
        f.archive
            .reply_headers(reply, &StalwartParser::default(), &f.blobs)
            .is_err()
    );
    assert!(db.query_row(
        "SELECT references_json IS NULL FROM messages WHERE id=?1",
        [reply.0 as i64],
        |row| row.get::<_, bool>(0)
    )?);
    assert_eq!(f.members(root)?, BTreeSet::from([root, reply]));
    db.execute_batch("DROP TRIGGER fail_graph;")?;
    f.archive
        .reply_headers(reply, &StalwartParser::default(), &f.blobs)?;
    assert_eq!(f.members(root)?, BTreeSet::from([root, reply]));
    Ok(())
}

#[test]
fn migration_restarts_after_a_failed_page_without_reading_blobs() -> Result {
    let mut f = Fixture::new()?;
    let mut ids = Vec::new();
    for n in 0..105 {
        ids.push(f.add(
            "a",
            &format!("{n}@example.org"),
            "References: <root@example.org>\r\n",
        )?);
    }
    let db = f.db()?;
    db.execute_batch(concat!(
        include_str!("fixtures/remove_sent_copy.sql"),
        "DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules;"
    ))?;
    db.execute_batch("DROP TABLE occurrence_keywords; DROP TABLE keyword_labels; DROP TABLE keyword_operations; ALTER TABLE move_operations DROP COLUMN kind; DROP TABLE folder_operations; DROP TABLE thread_pending; DROP TABLE thread_edges; DROP TABLE thread_members; DROP TABLE thread_nodes; PRAGMA user_version=13;")?;
    // A malformed retained header simulates failure in the second migration page.
    db.execute(
        "UPDATE messages SET references_json='invalid' WHERE id=?1",
        [ids[102].0 as i64],
    )?;
    assert!(Archive::open(f.dir.path()).is_err());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM thread_members", [], |row| row
            .get::<_, i64>(0))?,
        100
    );
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM thread_pending", [], |row| row
            .get::<_, i64>(0))?,
        5
    );
    db.execute(
        "UPDATE messages SET references_json='[\"root@example.org\"]' WHERE id=?1",
        [ids[102].0 as i64],
    )?;
    f.archive = Archive::open(f.dir.path())?;
    assert_eq!(f.members(ids[0])?, ids.iter().copied().collect());
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM thread_pending", [], |row| row
            .get::<_, i64>(0))?,
        0
    );
    // Recovering a legacy root-key guess replaces its provisional edges.
    db.execute(
        "UPDATE messages SET references_json=NULL,thread_key='wrong@example.org' WHERE id=?1",
        [ids[0].0 as i64],
    )?;
    db.execute_batch(concat!(
        include_str!("fixtures/remove_sent_copy.sql"),
        "DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules;"
    ))?;
    db.execute_batch("DROP TABLE occurrence_keywords; DROP TABLE keyword_labels; DROP TABLE keyword_operations; ALTER TABLE move_operations DROP COLUMN kind; DROP TABLE folder_operations; DROP TABLE thread_pending; DROP TABLE thread_edges; DROP TABLE thread_members; DROP TABLE thread_nodes; PRAGMA user_version=13;")?;
    f.archive = Archive::open(f.dir.path())?;
    assert_eq!(f.members(ids[0])?, BTreeSet::from([ids[0]]));
    f.archive
        .reply_headers(ids[0], &StalwartParser::default(), &f.blobs)?;
    assert_eq!(f.members(ids[0])?, ids.into_iter().collect());
    Ok(())
}
