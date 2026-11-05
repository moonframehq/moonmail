use {
    moonmail_core::{IndexChange, MailPredicate, MailQuery, MailRepository, SearchIndex},
    moonmail_mime::StalwartParser,
    moonmail_search::{SqliteIndex, parse_query},
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn boolean_search_filters_before_paging_and_live_metadata_and_saved_queries_survive_restart()
-> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let mut index = SqliteIndex::in_memory()?;
    let inbox = archive.mailbox("me@example.org", "INBOX")?;
    let other = archive.mailbox("me@example.org", "Other")?;
    let mut ids = Vec::new();
    for n in 0..205 {
        let raw = format!(
            "From: Alice <alice@example.org>\r\nTo: me@example.org\r\nCc: bob@example.org\r\nSubject: Design review\r\n\r\n{}",
            if n % 2 == 0 { "approved" } else { "pending" }
        );
        ids.push(archive.ingest(
            &StalwartParser::default(),
            &blobs,
            IncomingMessage {
                account: "me@example.org",
                mailbox: if n < 200 { inbox } else { other },
                origin: &n.to_string(),
                epoch: 1,
                uid: Some(n + 1),
                seen: false,
                starred: false,
                received_at: 1_700_000_000 + i64::from(n),
                raw: raw.as_bytes(),
            },
        )?);
    }
    for chunk in archive.index_work(1000)?.chunks(100) {
        index.apply(
            &chunk
                .iter()
                .map(|work| IndexChange::Upsert(work.id, &work.document))
                .collect::<Vec<_>>(),
        )?;
    }
    let source = "in:INBOX account:me@example.org (body:approved OR cc:nobody) NOT is:read subject:\"design review\"";
    let predicate = parse_query(source)?;
    archive.prepare_predicate(&index, &predicate)?;
    let mut query = MailQuery {
        search_matches: true,
        ..Default::default()
    };
    let mut found = Vec::new();
    loop {
        let page = archive.page(&query, 17)?;
        found.extend(page.rows.iter().map(|row| row.id));
        query.before = page.next;
        if query.before.is_none() {
            break;
        }
    }
    let expected: Vec<_> = ids[..200].iter().step_by(2).rev().copied().collect();
    assert_eq!(found, expected);
    archive.set_flag(ids[198], "seen", true)?;
    assert_eq!(
        archive.page(&query, 200)?.rows.len(),
        99,
        "metadata must stay live without reparsing text"
    );
    archive.prepare_predicate(&index, &parse_query("NOT in:INBOX")?)?;
    assert_eq!(archive.page(&query, 200)?.rows.len(), 5);
    query.mailbox = Some(inbox);
    assert!(
        archive.page(&query, 200)?.rows.is_empty(),
        "NOT cannot escape the outer folder scope"
    );
    query.mailbox = None;
    archive.prepare_predicate(&index, &parse_query("after:2023-11-14 before:2023-11-15")?)?;
    assert_eq!(archive.page(&query, 200)?.rows.len(), 200);
    archive.prepare_predicate(&index, &parse_query("from:bob OR to:bob")?)?;
    assert!(archive.page(&query, 200)?.rows.is_empty());
    archive.prepare_predicate(&index, &parse_query("cc:bob")?)?;
    assert!(archive.page(&query, 200)?.next.is_some());
    assert!(
        archive
            .prepare_predicate(&index, &MailPredicate::All(vec![]))
            .is_err()
    );
    assert_eq!(
        archive.page(&query, 200)?.rows.len(),
        200,
        "failed preparation preserves the previous query"
    );
    let saved = archive.save_search("Reviews", source, &predicate)?;
    assert_eq!(archive.save_search("Reviews", source, &predicate)?, saved);
    assert_eq!(archive.saved_searches()?.len(), 1);
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    let saved_query = archive.saved_search(saved)?;
    assert_eq!(saved_query.source, source);
    archive.prepare_predicate(&index, &saved_query.predicate)?;
    assert_eq!(archive.page(&query, 200)?.rows.len(), 99);
    archive.delete_search(saved)?;
    assert!(archive.saved_search(saved).is_err());
    Ok(())
}
