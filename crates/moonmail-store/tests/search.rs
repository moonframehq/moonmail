use {
    moonmail_core::{
        IndexChange, MailQuery, MailRepository, MessageId, SearchDocument, SearchIndex,
        SearchQuery, ViewFilter,
    },
    moonmail_mime::StalwartParser,
    moonmail_search::SqliteIndex,
    moonmail_store::{Archive, FileBlobs, IncomingMessage, SearchError},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn search_scopes_before_paging_and_grouping_without_a_global_hit_cap() -> Result {
    let directory = tempfile::tempdir()?;
    let mut archive = Archive::open(directory.path())?;
    let inbox = archive.mailbox("synthetic@example.org", "INBOX")?;
    let other = archive.mailbox("synthetic@example.org", "Other")?;
    // Synthetic metadata only: navigation must never need bodies or raw files.
    let mut connection = rusqlite::Connection::open(directory.path().join("archive.sqlite3"))?;
    let transaction = connection.transaction()?;
    for id in 1..=1205 {
        transaction.execute(
            "INSERT INTO messages(id,account,origin,blob,subject,sender,address,recipients,date,snippet,rfc_id,thread_key)
             VALUES (?1,'synthetic@example.org',?1,'synthetic','needle','Sender','sender@example.org','me@example.org',?1,'',?1,?2)",
            rusqlite::params![id, if id > 1100 { 1101 + ((id - 1101) / 2) * 2 } else { id }],
        )?;
        // Retired hits and hits in another folder both precede the inbox hits.
        if !(551..=1100).contains(&id) {
            transaction.execute(
                "INSERT INTO occurrences VALUES (?1,?2,1,?2,?3,0)",
                rusqlite::params![if id <= 550 { other } else { inbox }, id, id % 2 == 0],
            )?;
        }
    }
    transaction.commit()?;
    let mut index = SqliteIndex::in_memory()?;
    let document = SearchDocument {
        subject: "needle".into(),
        ..Default::default()
    };
    index.apply(
        &(1..=1204)
            .map(|id| IndexChange::Upsert(MessageId(id), &document))
            .collect::<Vec<_>>(),
    )?;
    let base = MailQuery {
        mailbox: Some(inbox),
        search_matches: true,
        ..Default::default()
    };
    assert!(
        archive.page(&base, 10).is_err(),
        "unprepared is not silently empty"
    );
    archive.prepare_search(&index, &["needle"])?;
    for threaded in [false, true] {
        for filter in [ViewFilter::All, ViewFilter::Unread] {
            let mut query = MailQuery {
                threaded,
                filter,
                ..base.clone()
            };
            let mut found = Vec::new();
            loop {
                let page = archive.page(&query, 17)?;
                assert!(page.rows.len() <= 17);
                for row in page.rows {
                    assert!((1101..=1204).contains(&row.id.0));
                    assert_eq!(
                        row.thread_count,
                        if threaded && filter == ViewFilter::All {
                            2
                        } else {
                            1
                        }
                    );
                    found.push(row.id);
                }
                match page.next {
                    Some(cursor) => query.before = Some(cursor),
                    None => break,
                }
            }
            let expected: Vec<_> = (1101..=1204)
                .rev()
                .filter(|id| !(threaded || filter == ViewFilter::Unread) || id % 2 == 1)
                .map(MessageId)
                .collect();
            assert_eq!(found, expected);
        }
    }
    // Scope stays live even while the cached text-match set is unchanged.
    connection.execute("DELETE FROM occurrences WHERE message_id=1204", [])?;
    assert_eq!(archive.page(&base, 1)?.rows[0].id, MessageId(1203));
    archive.prepare_search(&index, &["absent"])?;
    assert!(archive.page(&base, 10)?.rows.is_empty());
    Ok(())
}

struct InterruptedIndex;

impl SearchIndex for InterruptedIndex {
    type Error = std::io::Error;
    fn visit_terms(
        &self,
        _: &[moonmail_core::TextTerm],
        mut visitor: impl FnMut(usize, MessageId) -> bool,
    ) -> std::io::Result<()> {
        visitor(0, MessageId(999));
        Err(std::io::Error::other("synthetic interrupted scan"))
    }

    fn apply(&mut self, _: &[IndexChange<'_>]) -> std::io::Result<()> {
        unreachable!()
    }
    fn search(&self, _: SearchQuery<'_>) -> std::io::Result<Vec<MessageId>> {
        unreachable!()
    }
    fn visit_matches(
        &self,
        _: &[&str],
        mut visitor: impl FnMut(MessageId) -> bool,
    ) -> std::io::Result<()> {
        visitor(MessageId(999));
        Err(std::io::Error::other("synthetic interrupted scan"))
    }
}

#[test]
fn projection_upgrade_replays_cc_and_sender_names_and_failed_search_keeps_previous_set() -> Result {
    let directory = tempfile::tempdir()?;
    let mut archive = Archive::open(directory.path())?;
    let inbox = archive.mailbox("me@example.org", "INBOX")?;
    let id = archive.ingest(&StalwartParser::default(), &FileBlobs::new(directory.path()), IncomingMessage {
        account: "me@example.org", mailbox: inbox, origin: "synthetic", epoch: 1,
        uid: Some(1), seen: false, starred: false, received_at: 1,
        raw: b"From: Nickname <sender@example.org>\r\nTo: me@example.org\r\nCc: ccneedle@example.org\r\nSubject: Subject\r\n\r\nBody",
    })?;
    let sequence = archive.index_work(100)?[0].sequence;
    archive.acknowledge_index(sequence)?;
    drop(archive);
    let connection = rusqlite::Connection::open(directory.path().join("archive.sqlite3"))?;
    connection.execute_batch(include_str!("fixtures/remove_sent_copy.sql"))?;
    connection.execute_batch(
        "DROP TABLE folder_operations; DROP TABLE thread_pending; DROP TABLE thread_edges; DROP TABLE thread_members; DROP TABLE thread_nodes;
         DROP TABLE search_sources; DROP TABLE saved_searches;
         ALTER TABLE messages DROP COLUMN header_date;
         ALTER TABLE messages DROP COLUMN received_at;
         DROP TABLE identities;
         ALTER TABLE drafts DROP COLUMN identity_json;
         DROP INDEX outbox_sent_tracking;
         ALTER TABLE outbox DROP COLUMN rfc_id;
         ALTER TABLE outbox DROP COLUMN sent_status;
         ALTER TABLE outbox DROP COLUMN sent_message;
         ALTER TABLE outbox DROP COLUMN draft_json;
         ALTER TABLE outbox DROP COLUMN restored_draft_id;
         ALTER TABLE drafts DROP COLUMN attachments_json;
         DROP INDEX drafts_updated;
         ALTER TABLE messages DROP COLUMN reply_addresses_json;
         ALTER TABLE drafts DROP COLUMN cc;
         ALTER TABLE drafts DROP COLUMN bcc;
         ALTER TABLE drafts DROP COLUMN account;
         DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules;
         DROP TABLE occurrence_keywords; DROP TABLE keyword_labels; DROP TABLE keyword_operations;
         DROP TABLE move_operations;
         DROP INDEX mailbox_user_role;
         ALTER TABLE mailboxes DROP COLUMN server_role;
         ALTER TABLE mailboxes DROP COLUMN user_role;
         ALTER TABLE mailboxes DROP COLUMN active;
         ALTER TABLE messages DROP COLUMN references_json;
         ALTER TABLE messages DROP COLUMN in_reply_to;
         ALTER TABLE drafts DROP COLUMN references_json;
         PRAGMA user_version=2;",
    )?;
    drop(connection);
    let archive = Archive::open(directory.path())?;
    assert_eq!(
        archive.index_work(100)?.len(),
        1,
        "upgrade schedules replay"
    );
    drop(archive); // Crash before indexing/acknowledgment must not lose the replay.
    let mut archive = Archive::open(directory.path())?;
    let work = archive.index_work(100)?;
    assert_eq!(work.len(), 1);
    let mut index = SqliteIndex::in_memory()?;
    index.apply(&[IndexChange::Upsert(id, &work[0].document)])?;
    let query = MailQuery {
        search_matches: true,
        ..Default::default()
    };
    for term in ["ccneedle", "Nickname", "sender"] {
        archive.prepare_search(&index, &[term])?;
        assert_eq!(archive.page(&query, 10)?.rows[0].id, id);
    }
    assert!(matches!(
        archive.prepare_search(&InterruptedIndex, &["other"]),
        Err(SearchError::Index(_))
    ));
    assert_eq!(
        archive.page(&query, 10)?.rows[0].id,
        id,
        "partial replacement rolls back"
    );
    let predicate = moonmail_search::parse_query("cc:ccneedle")?;
    archive.prepare_predicate(&index, &predicate)?;
    assert!(matches!(
        archive.prepare_predicate(
            &InterruptedIndex,
            &moonmail_search::parse_query("body:other")?
        ),
        Err(SearchError::Index(_))
    ));
    assert_eq!(
        archive.page(&query, 10)?.rows[0].id,
        id,
        "failed typed-query preparation retains both terms and compiled metadata"
    );
    archive.acknowledge_index(sequence)?;
    drop(archive);
    let archive = Archive::open(directory.path())?;
    assert!(
        archive.index_work(100)?.is_empty(),
        "upgrade runs only once"
    );
    assert!(
        archive.page(&query, 10).is_err(),
        "temporary matches do not survive reopen"
    );
    Ok(())
}
