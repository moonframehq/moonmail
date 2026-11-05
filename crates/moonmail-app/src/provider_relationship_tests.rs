use super::{tests::data, *};

#[test]
fn expanded_threads_page_all_members_skip_controls_and_reject_stale_pages() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    let elsewhere = archive.mailbox("you@example.com", "Elsewhere")?;
    let mut expected = HashSet::new();
    for n in 0..240 {
        let raw = format!(
            "From: peer@example.org\r\nTo: you@example.com\r\nSubject: Synthetic {n}\r\nMessage-ID: <{n}@example.org>\r\nReferences: <root@example.org>\r\n\r\nBody"
        );
        let id = archive.ingest(
            &moonmail_mime::StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            moonmail_store::IncomingMessage {
                account: "you@example.com",
                mailbox: if n < 237 { inbox } else { elsewhere },
                origin: &n.to_string(),
                epoch: 0,
                uid: None,
                seen: true,
                starred: n % 2 == 0,
                received_at: n / 3,
                raw: raw.as_bytes(),
            },
        )?;
        if n < 237 {
            expected.insert(id.0);
        }
    }
    drop(archive);
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    mail.query.mailbox = Some(inbox);
    mail.query.threaded = true;
    mail.refresh()?;
    let representative = mail.page.rows[0].id;
    mail.handle(data(json!({"action":"expand","id":representative})))?;
    let first = mail.listing()?;
    assert_eq!(first[1]["thread_pager"], true);
    mail.select(representative, true)?;
    mail.handle(data(json!({"action":"move","value":"down"})))?;
    assert_eq!(
        mail.selected.as_ref().context("selection")?.summary.id.0,
        first[2]["id"].as_u64().context("child")?
    );
    let mut found = HashSet::from([representative.0]);
    let mut pages = 0;
    loop {
        let rows = mail.listing()?;
        for row in rows
            .iter()
            .filter(|row| row["depth"] == 1 && row["thread_pager"] != true)
        {
            let id = row["id"].as_u64().context("child id")?;
            assert_ne!(id, representative.0);
            assert!(found.insert(id));
        }
        pages += 1;
        let pager = rows
            .iter()
            .find(|row| row["thread_pager"] == true)
            .context("pager")?;
        if pager["thread_more"] != true {
            break;
        }
        let event =
            json!({"action":"thread_next","id":representative,"cursor":pager["thread_cursor"]});
        mail.checked.insert(representative);
        mail.handle(data(event.clone()))?;
        assert!(mail.checked.is_empty());
        assert!(
            mail.handle(data(event)).is_err(),
            "duplicate Next must not skip a page"
        );
    }
    assert_eq!(pages, 3);
    assert_eq!(found, expected);
    let cursor = mail
        .expanded_query(representative)
        .before
        .context("last cursor")?
        .id;
    mail.handle(data(
        json!({"action":"thread_first","id":representative,"cursor":cursor}),
    ))?;
    assert_eq!(mail.listing()?, first);
    mail.handle(data(json!({"action":"expand","id":representative})))?;
    assert!(
        mail.handle(data(
            json!({"action":"thread_next","id":representative,"cursor":0})
        ))
        .is_err()
    );
    mail.query.filter = ViewFilter::Starred;
    mail.refresh()?;
    let representative = mail.page.rows[0].id;
    mail.handle(data(json!({"action":"expand","id":representative})))?;
    assert!(
        mail.listing()?
            .iter()
            .filter(|row| row["thread_pager"] != true)
            .all(|row| row["starred"] == true)
    );
    mail.handle(data(json!({"action":"folder","id":elsewhere})))?;
    assert!(mail.expanded.is_empty());
    assert!(
        mail.handle(data(
            json!({"action":"thread_next","id":representative,"cursor":0})
        ))
        .is_err()
    );
    Ok(())
}

#[test]
fn related_navigation_keeps_anchor_and_resets_cursors_on_query_changes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    for n in 0..110 {
        let raw = format!(
            "From: peer@example.org\r\nTo: you@example.com\r\nSubject: Synthetic {n}\r\nMessage-ID: <{n}@example.org>\r\nReferences: <root@example.org>\r\n\r\nBody"
        );
        archive.ingest(
            &moonmail_mime::StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            moonmail_store::IncomingMessage {
                account: "you@example.com",
                mailbox: inbox,
                origin: &n.to_string(),
                epoch: 0,
                uid: None,
                seen: true,
                starred: false,
                received_at: n,
                raw: raw.as_bytes(),
            },
        )?;
    }
    drop(archive);
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    let anchor = mail.anchor.context("missing anchor")?;
    let first = mail
        .archive()?
        .related_page(&mail.related_query().context("missing query")?, 50)?;
    assert_eq!(first.rows.len(), 50);
    mail.handle(data(json!({"action":"related_next","anchor":anchor})))?;
    assert!(mail.related_before.is_some());
    let second = mail
        .archive()?
        .related_page(&mail.related_query().context("missing query")?, 50)?;
    assert!(
        second
            .rows
            .iter()
            .all(|row| first.rows.iter().all(|old| old.id != row.id))
    );
    mail.handle(data(json!({"action":"related","id":second.rows[0].id})))?;
    assert_eq!(mail.anchor, Some(anchor));
    assert!(mail.related_before.is_some());
    assert!(
        mail.handle(data(
            json!({"action":"related_next","anchor":second.rows[0].id})
        ))
        .is_err()
    );
    mail.handle(data(
        json!({"action":"related_sort","anchor":anchor,"field":"subject"}),
    ))?;
    assert!(mail.related_before.is_none());
    assert_eq!(mail.related_sort, SortField::Subject);
    assert!(mail.related_ascending);
    mail.handle(data(json!({"action":"related_next","anchor":anchor})))?;
    mail.handle(data(json!({"action":"related_mode","anchor":anchor})))?;
    assert!(mail.related_before.is_none());
    assert_eq!(mail.relationship, Relationship::Thread);
    mail.handle(data(json!({"action":"related_next","anchor":anchor})))?;
    mail.handle(data(json!({"action":"select","id":second.rows[0].id})))?;
    assert_eq!(mail.anchor, Some(second.rows[0].id));
    assert!(mail.related_before.is_none());
    Ok(())
}
