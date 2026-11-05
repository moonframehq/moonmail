use super::{tests::data, *};

#[test]
fn unread_navigation_reveals_hidden_children_and_keeps_search_folder_and_flag_scope() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    let outside = archive.mailbox("you@example.com", "Outside")?;
    let mut ids = Vec::new();
    for n in 0..155 {
        let (parent, date, mailbox, starred, subject) = match n {
            0 => ("", 1000, inbox, true, "Keep root"),
            1..=150 => (
                "References: <0@example.org>\r\n",
                n,
                inbox,
                true,
                "Keep reply",
            ),
            151 => ("", 2000, inbox, true, "Keep next root"),
            152 => (
                "References: <151@example.org>\r\n",
                1800,
                inbox,
                true,
                "Keep next reply",
            ),
            153 => ("", 1900, outside, true, "Keep outside"),
            _ => ("", 1900, inbox, false, "Keep unflagged"),
        };
        let raw = format!(
            "From: Peer <peer@example.org>\r\nSubject: {subject}\r\nMessage-ID: <{n}@example.org>\r\n{parent}\r\nSynthetic"
        );
        ids.push(archive.ingest(
            &moonmail_mime::StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            moonmail_store::IncomingMessage {
                account: "you@example.com",
                mailbox,
                origin: &n.to_string(),
                epoch: 0,
                uid: None,
                seen: !matches!(n, 150 | 152 | 153 | 154),
                starred,
                received_at: date,
                raw: raw.as_bytes(),
            },
        )?);
    }
    drop(archive);
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    mail.preferences.mark_read = false;
    while mail.index_pending()? {}
    mail.handle(data(json!({"action":"search","value":"subject:keep"})))?;
    mail.query.mailbox = Some(inbox);
    mail.query.filter = ViewFilter::Starred;
    mail.query.threaded = true;
    mail.refresh()?;
    assert!(
        mail.page.rows.iter().all(|row| row.unread),
        "read roots retain their unread-child indicators"
    );
    mail.select(ids[151], true)?;
    mail.handle(data(json!({"action":"seek_unread","value":"down"})))?;
    assert_eq!(
        mail.selected.as_ref().context("selection")?.summary.id,
        ids[150]
    );
    assert!(
        mail.listing()?
            .iter()
            .any(|row| row["id"] == ids[150].0 && row["thread_pager"] != true)
    );
    assert!(
        mail.expanded.get(&ids[0]).is_some_and(Option::is_some),
        "hidden unread child has its own bounded expansion page"
    );
    mail.handle(data(json!({"action":"seek_unread","value":"up"})))?;
    assert_eq!(
        mail.selected.as_ref().context("selection")?.summary.id,
        ids[152]
    );
    mail.handle(data(json!({"action":"seek_unread","value":"up"})))?;
    assert_eq!(
        mail.selected.as_ref().context("selection")?.summary.id,
        ids[152],
        "no wrapping at start"
    );
    assert_eq!(mail.query.mailbox, Some(inbox));
    assert_eq!(mail.query.filter, ViewFilter::Starred);
    assert!(mail.query.search_matches);
    mail.handle(data(json!({"action":"seek_thread","value":"down"})))?;
    assert_eq!(
        mail.selected.as_ref().context("selection")?.summary.id,
        ids[0]
    );
    assert!(
        mail.handle(data(json!({"action":"thread_read","id":ids[151]})))
            .is_err(),
        "stale thread command"
    );
    mail.handle(data(json!({"action":"thread_read","id":ids[0]})))?;
    assert!(!mail.archive()?.detail(ids[150])?.summary.unread);
    assert!(mail.archive()?.detail(ids[152])?.summary.unread);
    mail.handle(data(json!({"action":"thread_unread","id":ids[0]})))?;
    assert!(mail.archive()?.detail(ids[150])?.summary.unread);
    assert!(mail.selected.as_ref().context("selection")?.summary.unread);
    // Flat mode traverses unread messages with the same active scopes.
    mail.query.threaded = false;
    mail.select(ids[152], true)?;
    mail.handle(data(json!({"action":"seek_unread","value":"down"})))?;
    assert_eq!(
        mail.selected.as_ref().context("selection")?.summary.id,
        ids[0]
    );
    assert!(!mail.query.threaded);
    Ok(())
}

#[test]
fn keyboard_crosses_pages_and_previous_reverses_group_activity_and_text_sort_keys() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    for n in 0..163 {
        let raw = format!(
            "From: Peer {} <peer@example.org>\r\nSubject: Review {}\r\nMessage-ID: <{n}@example.org>\r\n\r\nSynthetic",
            n % 13,
            n % 9
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
                starred: true,
                received_at: n / 3,
                raw: raw.as_bytes(),
            },
        )?;
    }
    archive.ingest(&moonmail_mime::StalwartParser::default(),&FileBlobs::new(dir.path()),moonmail_store::IncomingMessage {account:"you@example.com",mailbox:inbox,origin:"late",epoch:0,uid:None,seen:true,starred:true,received_at:9999,raw:b"From: Peer <peer@example.org>\r\nSubject: Late review\r\nReferences: <0@example.org>\r\nMessage-ID: <late@example.org>\r\n\r\nSynthetic"})?;
    drop(archive);
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    mail.preferences.mark_read = false;
    while mail.index_pending()? {}
    mail.handle(data(json!({"action":"search","value":"subject:review"})))?;
    mail.query.mailbox = Some(inbox);
    mail.query.filter = ViewFilter::Starred;
    for threaded in [false, true] {
        for sort in [SortField::Date, SortField::Sender, SortField::Subject] {
            for ascending in [false, true] {
                mail.query.threaded = threaded;
                mail.query.sort = sort;
                mail.query.ascending = ascending;
                mail.query.before = None;
                mail.refresh()?;
                let ids = |mail: &Mail| mail.page.rows.iter().map(|row| row.id).collect::<Vec<_>>();
                let first = ids(&mail);
                let last = first[74];
                mail.select(last, true)?;
                mail.handle(data(json!({"action":"move","value":"down"})))?;
                let second = ids(&mail);
                assert_eq!(
                    mail.selected.as_ref().context("selection")?.summary.id,
                    second[0]
                );
                mail.handle(data(json!({"action":"move","value":"up"})))?;
                assert_eq!(ids(&mail), first);
                assert_eq!(
                    mail.selected.as_ref().context("selection")?.summary.id,
                    last
                );
                mail.handle(data(json!({"action":"next"})))?;
                assert_eq!(ids(&mail), second);
                mail.handle(data(json!({"action":"next"})))?;
                assert!(mail.page.next.is_none());
                assert!(
                    mail.page.last.is_some(),
                    "terminal pages retain their cursor bounds"
                );
                mail.handle(data(json!({"action":"previous"})))?;
                assert_eq!(ids(&mail), second);
                mail.handle(data(json!({"action":"previous"})))?;
                assert_eq!(ids(&mail), first);
                mail.handle(data(json!({"action":"previous"})))?;
                assert_eq!(ids(&mail), first);
                assert_eq!(mail.query.mailbox, Some(inbox));
                assert_eq!(mail.query.filter, ViewFilter::Starred);
            }
        }
    }
    Ok(())
}
