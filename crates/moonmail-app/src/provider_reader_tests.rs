use super::{tests::data, *};

#[test]
fn reader_actions_bind_links_to_selection_and_rebuild_html_without_remote_fetches() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    let raw = b"From: sender@example.org\r\nContent-Type: text/html\r\n\r\n<p>Visit <a href=\"https://example.org/docs\">documentation</a>.</p><img src=\"https://tracker.example/pixel\">";
    let id = archive.ingest(
        &moonmail_mime::StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        moonmail_store::IncomingMessage {
            account: "you@example.com",
            mailbox: inbox,
            origin: "links",
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 1,
            raw,
        },
    )?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    mail.reader_dir = Some(tempfile::tempdir_in(dir.path())?);
    mail.select(id, true)?;
    assert_eq!(mail.reader_link_target(id, 0)?, "https://example.org/docs");
    assert!(mail.reader_link_target(MessageId(id.0 + 1), 0).is_err());
    assert!(mail.reader_link_target(id, 1000).is_err());
    let before = mail.html_url.clone();
    mail.handle(data(json!({"action":"reader_images","message":id})))?;
    assert_ne!(mail.html_url, before);
    assert!(mail.reader_images.starts_with("0 embedded images"));
    let mut url = url::Url::parse(&mail.html_url)?;
    url.set_query(None);
    let html = std::fs::read_to_string(
        url.to_file_path()
            .map_err(|_| anyhow::anyhow!("invalid URL"))?,
    )?;
    assert!(!html.contains("tracker.example") && !html.contains("href="));
    mail.select(id, true)?;
    assert!(mail.reader_images.is_empty());
    Ok(())
}
