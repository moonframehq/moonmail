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
    assert!(
        mail.handle(data(json!({"action":"reader_remote_images","message":id})))
            .is_err()
    ); // offline demo
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

#[test]
fn settings_switch_the_selected_reader_and_survive_restart() -> Result<()> {
    use crate::reader_backend::Renderer;

    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    moonmail_store::seed_demo(&mut archive, &FileBlobs::new(dir.path()))?;
    drop(archive);
    let mut mail =
        Mail::new(dir.path().into(), true).with_font_config(dir.path().join("style.luau"));
    mail.initialize()?;
    let id = mail
        .selected
        .as_ref()
        .context("Expected a selection")?
        .summary
        .id;
    let save = |renderer: Option<&str>| {
        let mut request = json!({"action":"settings_save","interface_family":"Helvetica","list_family":"Menlo","font_size":"12","reader_family":"Helvetica","reader_size":"12","mark_read":false});
        if let Some(value) = renderer {
            request["reader_renderer"] = json!(value);
        }
        data(request)
    };
    assert_eq!(mail.preferences.reader_renderer, Renderer::Webkit);
    mail.handle(save(Some("native")))?;
    assert!(mail.plain_url.is_empty() && mail.html_url.is_empty());
    let revision = mail.reader_revision;
    assert!(mail.documents.read(revision, false).is_some());
    assert!(mail.documents.read(revision, true).is_some());
    assert_eq!(mail.selected.as_ref().map(|s| s.summary.id), Some(id));
    mail.handle(save(None))?;
    assert!(mail.documents.read(revision, true).is_none());
    assert!(mail.handle(save(Some("invalid"))).is_err());
    assert_eq!(
        crate::fonts::Preferences::load(dir.path())?.reader_renderer,
        Renderer::Native
    );
    drop(mail);
    let mut restarted =
        Mail::new(dir.path().into(), true).with_font_config(dir.path().join("style.luau"));
    restarted.initialize()?;
    assert!(
        restarted
            .documents
            .read(restarted.reader_revision, true)
            .is_some()
    );
    assert_eq!(
        std::fs::read_dir(restarted.reader_dir.as_ref().context("reader dir")?)?.count(),
        0
    );
    restarted.handle(save(Some("webkit")))?;
    assert!(restarted.plain_url.starts_with("file:"));
    assert!(
        restarted
            .documents
            .read(restarted.reader_revision, true)
            .is_none()
    );
    Ok(())
}

#[test]
fn remote_image_consent_follows_selection_and_saved_settings_in_both_readers() -> Result<()> {
    use {
        crate::reader_backend::Renderer,
        std::{
            sync::atomic::{AtomicBool, AtomicUsize, Ordering},
            time::{Duration, Instant},
        },
    };

    static DOWNLOADS: AtomicUsize = AtomicUsize::new(0);
    fn download(urls: Vec<String>, _: &AtomicBool) -> crate::reader_resources::EmbeddedImages {
        DOWNLOADS.fetch_add(1, Ordering::Relaxed);
        crate::reader_resources::EmbeddedImages {
            resources: urls.into_iter().map(|u| (u, "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAOCT3ioAAAAASUVORK5CYII=".to_string())).collect(),
            skipped: 0,
        }
    }
    fn finish(mail: &mut Mail) -> Result<()> {
        let end = Instant::now() + Duration::from_secs(3);
        while !mail.poll_images()? {
            anyhow::ensure!(Instant::now() < end, "image worker timed out");
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(())
    }
    fn visible(mail: &Mail) -> Result<usize> {
        match mail.preferences.reader_renderer {
            Renderer::Native => Ok(mail
                .documents
                .read(mail.reader_revision, true)
                .context("missing document")?
                .image_count()),
            Renderer::Webkit => {
                let path = url::Url::parse(&mail.html_url)?
                    .to_file_path()
                    .map_err(|_| anyhow::anyhow!("bad URL"))?;
                let html = std::fs::read_to_string(path)?;
                assert!(!html.contains("images.example"));
                assert!(html.contains("img-src data:"));
                Ok(html.matches("src=\"data:image/png;base64,").count())
            },
        }
    }
    let settings = |enabled: bool| {
        data(
            json!({"action":"settings_save","load_remote_images":enabled,"interface_family":"Helvetica","list_family":"Menlo","font_size":"12","reader_family":"Helvetica","reader_size":"12","mark_read":false}),
        )
    };
    for renderer in [Renderer::Native, Renderer::Webkit] {
        let dir = tempfile::tempdir()?;
        let mut mail =
            Mail::new(dir.path().into(), false).with_font_config(dir.path().join("style.luau"));
        let mut archive = Archive::open(dir.path())?;
        let inbox = archive.mailbox("me@example.org", "INBOX")?;
        let mut ids = Vec::new();
        for name in ["first", "second"] {
            let raw = format!(
                "From: sender@example.org\r\nSubject: {name}\r\nContent-Type: text/html\r\n\r\n<img src=\"https://images.example/{name}\">"
            );
            ids.push(archive.ingest(
                &moonmail_mime::StalwartParser::default(),
                &FileBlobs::new(dir.path()),
                moonmail_store::IncomingMessage {
                    account: "me@example.org",
                    mailbox: inbox,
                    origin: name,
                    epoch: 0,
                    uid: None,
                    seen: true,
                    starred: false,
                    received_at: 1,
                    raw: raw.as_bytes(),
                },
            )?);
        }
        mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
        mail.archive = Some(archive);
        mail.reader_dir = Some(tempfile::tempdir_in(dir.path())?);
        mail.preferences.reader_renderer = renderer;
        mail.image_state.test_download = Some(download);
        let start = DOWNLOADS.load(Ordering::Relaxed);
        let load = |id| data(json!({"action":"reader_remote_images","message":id}));
        mail.select(ids[0], true)?;
        assert_eq!(visible(&mail)?, 0);
        assert!(!mail.poll_images()?);
        assert!(mail.handle(load(ids[1])).is_err());
        assert_eq!(DOWNLOADS.load(Ordering::Relaxed), start);
        mail.handle(load(ids[0]))?;
        mail.handle(load(ids[0]))?;
        finish(&mut mail)?;
        assert_eq!(visible(&mail)?, 1);
        assert_eq!(DOWNLOADS.load(Ordering::Relaxed), start + 1);
        mail.select(ids[1], true)?;
        assert_eq!(visible(&mail)?, 0);
        mail.handle(load(ids[1]))?;
        mail.select(ids[0], true)?;
        finish(&mut mail)?; // stale completion, even when the download succeeds
        assert_eq!(visible(&mail)?, 0);
        mail.handle(settings(true))?;
        assert!(crate::fonts::Preferences::load(dir.path())?.load_remote_images);
        finish(&mut mail)?;
        assert_eq!(visible(&mail)?, 1);
        mail.select(ids[1], true)?;
        finish(&mut mail)?;
        assert_eq!(visible(&mail)?, 1);
        mail.select(ids[0], true)?;
        mail.handle(settings(false))?;
        finish(&mut mail)?;
        assert_eq!(visible(&mail)?, 0);
        assert!(!crate::fonts::Preferences::load(dir.path())?.load_remote_images);
        mail.select(ids[1], true)?;
        assert!(!mail.poll_images()?);
        assert_eq!(visible(&mail)?, 0);
    }
    Ok(())
}
