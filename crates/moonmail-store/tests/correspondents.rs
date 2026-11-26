use {
    moonmail_core::MailboxRole,
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn ingest(
    archive: &mut Archive,
    blobs: &FileBlobs,
    mailbox: i64,
    n: u32,
    from: &str,
    to: &str,
) -> Result {
    let raw = format!(
        "From: {from}\r\nTo: {to}\r\nSubject: Synthetic {n}\r\nMessage-ID: <{n}@example.org>\r\n\r\nSynthetic body"
    );
    archive.ingest(
        &StalwartParser::default(),
        blobs,
        IncomingMessage {
            account: "you@example.org",
            mailbox,
            origin: &format!("{mailbox}:{n}"),
            epoch: 1,
            uid: Some(n + 1),
            seen: true,
            starred: false,
            received_at: i64::from(n),
            raw: raw.as_bytes(),
        },
    )?;
    Ok(())
}

#[test]
fn correspondents_need_mail_both_ways_and_ignore_junk_and_trash() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("you@example.org", "INBOX")?;
    let sent = archive.mailbox("you@example.org", "Sent")?;
    let trash = archive.mailbox("you@example.org", "Trash")?;
    archive.mailbox_server_role(trash, Some(MailboxRole::Trash))?;
    let you = "You <you@example.org>";
    let clara = "Clara Jensen <clara@example.org>";
    let julian = "Julian Park <julian@example.org>";
    ingest(&mut archive, &blobs, inbox, 1, clara, you)?;
    ingest(&mut archive, &blobs, sent, 2, you, clara)?;
    ingest(&mut archive, &blobs, inbox, 3, clara, you)?;
    ingest(&mut archive, &blobs, inbox, 4, julian, you)?;
    ingest(&mut archive, &blobs, sent, 5, you, julian)?;
    // Newsletters never answered and trashed mail do not make a favorite.
    for n in 6..12 {
        ingest(
            &mut archive,
            &blobs,
            inbox,
            n,
            "News <news@example.net>",
            you,
        )?;
    }
    for n in 12..20 {
        ingest(&mut archive, &blobs, trash, n, you, julian)?;
    }
    let people = archive.frequent_correspondents(10)?;
    let summary: Vec<_> = people
        .iter()
        .map(|person| {
            (
                person.address.as_str(),
                person.name.as_str(),
                person.sent,
                person.received,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("clara@example.org", "Clara Jensen", 1, 2),
            ("julian@example.org", "Julian Park", 1, 1)
        ]
    );
    assert_eq!(archive.frequent_correspondents(1)?.len(), 1);
    Ok(())
}
