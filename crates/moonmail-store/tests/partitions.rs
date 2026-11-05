use {
    moonmail_core::{IndexChange, MailPredicate, MailQuery, MailRepository, SearchIndex},
    moonmail_mime::StalwartParser,
    moonmail_search::{SqliteIndex, parse_query},
    moonmail_store::{Archive, FileBlobs, IncomingMessage, PartitionField},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn partitions_cover_all_groups_count_distinct_messages_and_open_exact_multivalued_addresses()
-> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("a", "INBOX")?;
    let other = archive.mailbox("a", "Other")?;
    let mut ids = Vec::new();
    for n in 0..205 {
        ids.push(archive.ingest_with_keywords(&StalwartParser::default(),&blobs,IncomingMessage{
            account:"a",mailbox:inbox,origin:&n.to_string(),epoch:0,uid:None,seen:n%2==0,starred:false,received_at:n,
            raw:format!("From: Person <Person{n:03}@EXAMPLE.ORG>\r\nTo: Team <Team@EXAMPLE.ORG>\r\nCc: Team <Team@example.org>, Other <other@example.org>\r\nSubject: Keep\r\n\r\nSynthetic").as_bytes(),
        },&["work".into(),format!("project-{n:03}")])?);
    }
    archive.queue_copies(
        &[ids[0]],
        Some(inbox),
        moonmail_store::MoveTarget::Folder(other),
    )?;
    let mut index = SqliteIndex::in_memory()?;
    for work in archive.index_work(500)?.chunks(100) {
        index.apply(
            &work
                .iter()
                .map(|e| IndexChange::Upsert(e.id, &e.document))
                .collect::<Vec<_>>(),
        )?;
    }
    let base = parse_query("subject:Keep account:a")?;
    archive.prepare_predicate(&index, &base)?;
    let mut after = None;
    let mut groups = Vec::new();
    loop {
        let page = archive.partition_page(PartitionField::Sender, after.as_deref(), 17)?;
        assert!(page.rows.len() <= 17);
        groups.extend(page.rows);
        after = page.next;
        if after.is_none() {
            break;
        }
    }
    assert_eq!(groups.len(), 205);
    assert!(groups.iter().all(|g| g.total == 1));
    assert_eq!(groups[0].value, "Person000@example.org");
    let recipients = archive.partition_page(PartitionField::Recipient, None, 100)?;
    assert_eq!(recipients.rows.len(), 2);
    assert!(
        recipients
            .rows
            .iter()
            .all(|g| g.total == 205 && g.unread == 102),
        "same address in To and Cc and duplicate folder occurrences count once"
    );
    let keyword = archive.partition_page(PartitionField::Keyword, Some("project-999"), 100)?;
    assert_eq!(keyword.rows.len(), 1);
    assert_eq!(keyword.rows[0].value, "work");
    assert_eq!(keyword.rows[0].total, 205);
    let query = MailQuery {
        search_matches: true,
        ..Default::default()
    };
    archive.prepare_predicate(
        &index,
        &parse_query("sender-address:Person000@EXAMPLE.ORG")?,
    )?;
    assert_eq!(archive.page(&query, 100)?.rows.len(), 1);
    archive.prepare_predicate(
        &index,
        &parse_query("sender-address:person000@example.org")?,
    )?;
    assert!(
        archive.page(&query, 100)?.rows.is_empty(),
        "local-part case is preserved"
    );
    archive.prepare_predicate(&index, &parse_query("recipient-address:other@EXAMPLE.ORG")?)?;
    assert!(archive.page(&query, 100)?.next.is_some());
    archive.prepare_predicate(
        &index,
        &parse_query(
            "sender-address:Person000@example.org OR sender-address:Person001@example.org",
        )?,
    )?;
    assert_eq!(
        archive
            .partition_page(PartitionField::Keyword, None, 100)?
            .rows
            .len(),
        3,
        "partition after the entire predicate, not global keywords"
    );
    archive.set_keywords(&[ids[0], ids[1]], "work", false)?;
    assert_eq!(
        archive
            .partition_page(PartitionField::Keyword, None, 100)?
            .rows
            .len(),
        2,
        "membership stays live without reindexing"
    );
    Ok(())
}

#[test]
fn partition_configuration_survives_parent_edits_and_restart_and_cascades_on_delete() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let source = archive.save_search("Source", "account:a", &MailPredicate::Account("a".into()))?;
    let child =
        archive.save_search_from("Child", "is:unread", &MailPredicate::Unread, &[source])?;
    archive.set_search_partition(child, Some(PartitionField::Recipient))?;
    archive.save_search("Source", "is:starred", &MailPredicate::Starred)?;
    drop(archive);
    let archive = Archive::open(dir.path())?;
    assert_eq!(
        archive.search_partition(child)?,
        Some(PartitionField::Recipient)
    );
    archive.delete_search(child)?;
    assert_eq!(archive.search_partition(child)?, None);
    assert!(
        archive
            .set_search_partition(child, Some(PartitionField::Sender))
            .is_err()
    );
    assert!(
        archive
            .partition_page(PartitionField::Sender, None, 100)
            .is_err()
    );
    drop(archive);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(include_str!("fixtures/remove_sent_copy.sql"))?;
    db.execute_batch("DROP TABLE search_partitions; PRAGMA user_version=18;")?;
    let archive = Archive::open(dir.path())?;
    assert_eq!(archive.saved_searches()?.len(), 1);
    assert_eq!(archive.search_partition(source)?, None);
    Ok(())
}
