use {
    moonmail_core::{
        MailRepository, MailboxRole, MessageId, RelatedQuery, Relationship, SortField,
    },
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
    std::collections::HashSet,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn ingest(
    archive: &mut Archive,
    blobs: &FileBlobs,
    account: &str,
    mailbox: i64,
    n: u32,
    root: &str,
) -> Result<MessageId> {
    let name = if n.is_multiple_of(2) { "Peer" } else { "peer" };
    let raw = format!(
        "From: {name} <peer@example.org>\r\nTo: you@example.org\r\nSubject: Group {}\r\nMessage-ID: <{n}-{mailbox}@example.org>\r\nReferences: <{root}>\r\n\r\nSynthetic body excluded from navigation",
        n % 7
    );
    Ok(archive.ingest(
        &StalwartParser::default(),
        blobs,
        IncomingMessage {
            account,
            mailbox,
            origin: &format!("{mailbox}:{n}"),
            epoch: 1,
            uid: Some(n + 1),
            seen: true,
            starred: false,
            received_at: i64::from(n / 3),
            raw: raw.as_bytes(),
        },
    )?)
}

#[test]
fn related_pages_cover_every_match_with_ties_and_keep_a_retired_anchor_separate() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("you@example.org", "INBOX")?;
    let old = archive.mailbox("you@example.org", "Old")?;
    let anchor = ingest(
        &mut archive,
        &blobs,
        "you@example.org",
        old,
        0,
        "root@example.org",
    )?;
    archive.reconcile(old, 2, &[])?;
    let mut expected = HashSet::new();
    for n in 1..=237 {
        expected.insert(ingest(
            &mut archive,
            &blobs,
            "you@example.org",
            inbox,
            n,
            "root@example.org",
        )?);
    }
    for relationship in [Relationship::Correspondence, Relationship::Thread] {
        for sort in [SortField::Date, SortField::Sender, SortField::Subject] {
            for ascending in [false, true] {
                let mut query = RelatedQuery {
                    anchor,
                    relationship,
                    sort,
                    ascending,
                    before: None,
                };
                let mut seen = HashSet::new();
                let mut previous = None;
                loop {
                    let page = archive.related_page(&query, 17)?;
                    assert_eq!(page.anchor.id, anchor);
                    assert!(page.rows.len() <= 17);
                    for row in &page.rows {
                        assert_ne!(row.id, anchor);
                        assert!(seen.insert(row.id));
                        let key = match sort {
                            SortField::Date => format!("{:020}", row.date),
                            SortField::Sender => row.sender.to_ascii_lowercase(),
                            SortField::Subject => row.subject.to_ascii_lowercase(),
                        };
                        let position = (key, row.id);
                        if let Some(previous) = previous {
                            assert!(if ascending {
                                previous < position
                            } else {
                                previous > position
                            });
                        }
                        previous = Some(position);
                    }
                    query.before = page.next;
                    if query.before.is_none() {
                        break;
                    }
                }
                assert_eq!(seen, expected);
            }
        }
    }
    Ok(())
}

#[test]
fn correspondence_roles_override_display_names_and_old_cursor_allows_deletion_without_duplicates()
-> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("you@example.org", "INBOX")?;
    let anchor = ingest(
        &mut archive,
        &blobs,
        "you@example.org",
        inbox,
        0,
        "root@example.org",
    )?;
    let junk = archive.mailbox("you@example.org", "Indésirables")?;
    archive.mailbox_server_role(junk, Some(MailboxRole::Junk))?;
    let excluded = ingest(
        &mut archive,
        &blobs,
        "you@example.org",
        junk,
        2,
        "other@example.org",
    )?;
    let named_trash = archive.mailbox("you@example.org", "Trash")?;
    archive.set_mailbox_role(named_trash, Some(MailboxRole::Archive))?;
    let included = ingest(
        &mut archive,
        &blobs,
        "you@example.org",
        named_trash,
        3,
        "other@example.org",
    )?;
    let other = archive.mailbox("other@example.org", "INBOX")?;
    let cross_account = ingest(
        &mut archive,
        &blobs,
        "other@example.org",
        other,
        4,
        "other@example.org",
    )?;
    let mut query = RelatedQuery {
        anchor,
        relationship: Relationship::Correspondence,
        sort: SortField::Date,
        ascending: false,
        before: None,
    };
    let first = archive.related_page(&query, 1)?;
    assert_eq!(first.rows[0].id, cross_account);
    query.before = first.next;
    archive.reconcile(other, 2, &[])?;
    let second = archive.related_page(&query, 1)?;
    assert_eq!(second.rows[0].id, included);
    assert_ne!(second.rows[0].id, excluded);
    assert!(second.next.is_none());
    query.before = None;
    query.relationship = Relationship::Thread;
    assert!(archive.related_page(&query, 20)?.rows.is_empty());
    Ok(())
}
