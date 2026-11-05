use {
    moonmail_core::{BlobStore, MailQuery, MailRepository, MessageId},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, Draft, FileBlobs, IncomingMessage},
    std::error::Error,
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn ingest(
    archive: &mut Archive,
    blobs: &FileBlobs,
    mailbox: i64,
    (epoch, uid): (u32, u32),
    from: &str,
    to: &str,
    date: i64,
) -> Result<MessageId> {
    let raw = format!(
        "From: {from}\r\nTo: {to}\r\nSubject: Test {uid}\r\nMessage-ID: <{epoch}-{uid}@example.org>\r\n\r\nSynthetic body {uid}"
    );
    Ok(archive.ingest(
        &StalwartParser::default(),
        blobs,
        IncomingMessage {
            account: "me@example.com",
            mailbox,
            origin: &format!("{epoch}:{uid}"),
            epoch,
            uid: Some(uid),
            seen: false,
            starred: false,
            received_at: date,
            raw: raw.as_bytes(),
        },
    )?)
}

#[test]
fn durable_paging_correspondence_and_uid_epoch_reconciliation() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("me@example.com", "INBOX")?;
    let first = ingest(
        &mut archive,
        &blobs,
        inbox,
        (1, 1),
        "a@example.org",
        "me@example.com",
        100,
    )?;
    let second = ingest(
        &mut archive,
        &blobs,
        inbox,
        (1, 2),
        "me@example.com",
        "a@example.org",
        100,
    )?;
    ingest(
        &mut archive,
        &blobs,
        inbox,
        (1, 3),
        "b@example.org",
        "me@example.com",
        100,
    )?;
    let related = archive.correspondence(first, 100)?;
    assert_eq!(
        related.iter().map(|row| row.id).collect::<Vec<_>>(),
        vec![second, first]
    );
    let page = archive.page(&MailQuery::default(), 2)?;
    let tail = archive.page(
        &MailQuery {
            before: page.next,
            ..Default::default()
        },
        2,
    )?;
    assert_eq!(tail.rows.len(), 1);
    assert_eq!(tail.rows[0].id, first);
    assert!(page.rows.iter().all(|row| row.id != first));
    let blob = archive.detail(first)?.blob;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert!(blobs.get(&blob)?.starts_with(b"From:"));
    assert_eq!(archive.total()?, 3);
    archive.set_flag(first, "seen", true)?;
    archive.reconcile(inbox, 1, &[(1, false, false), (2, false, false)])?;
    assert!(
        !archive.detail(first)?.summary.unread,
        "pending local flags must survive stale server snapshots"
    );
    assert_eq!(archive.page(&MailQuery::default(), 100)?.rows.len(), 2);
    archive.reconcile(inbox, 2, &[(1, false, false)])?;
    assert!(archive.page(&MailQuery::default(), 100)?.rows.is_empty());
    let replacement = ingest(
        &mut archive,
        &blobs,
        inbox,
        (2, 1),
        "a@example.org",
        "me@example.com",
        200,
    )?;
    assert_ne!(
        first, replacement,
        "UID reuse in a new epoch is a new identity"
    );
    assert_eq!(archive.total()?, 4, "old raw mail stays archived");
    Ok(())
}

#[test]
fn journal_replay_and_outbox_crash_recovery() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let mut archive = Archive::open(dir.path())?;
    let mailbox = archive.mailbox("me@example.com", "INBOX")?;
    ingest(
        &mut archive,
        &blobs,
        mailbox,
        (1, 1),
        "a@example.org",
        "me@example.com",
        10,
    )?;
    let work = archive.index_work(100)?;
    assert_eq!(work.len(), 1);
    let seq = work[0].sequence;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert_eq!(
        archive.index_work(100)?[0].sequence,
        seq,
        "unacknowledged changes replay after restart"
    );
    archive.acknowledge_index(seq)?;
    assert!(archive.index_work(100)?.is_empty());
    archive.reset_index()?;
    assert_eq!(archive.index_work(100)?.len(), 1);
    let draft = Draft {
        id: "draft-one".into(),
        recipients: "a@example.org".into(),
        subject: "Hello".into(),
        body: "Synthetic".into(),
        reply_id: String::new(),
        references: Vec::new(),
        ..Default::default()
    };
    archive.save_draft(&draft)?;
    let blob = blobs.put(b"From: me@example.com\r\n\r\nSynthetic")?;
    let id = archive.queue(&draft, "me@example.com", &blob, &["a@example.org".into()])?;
    assert_eq!(
        archive.queue(&draft, "me@example.com", &blob, &["a@example.org".into()])?,
        id
    );
    assert!(archive.latest_draft()?.is_none());
    assert!(archive.claim_send(id)?);
    assert!(!archive.claim_send(id)?);
    drop(archive);
    let archive = Archive::open(dir.path())?;
    archive.recover_interrupted_sends()?;
    assert_eq!(archive.outbox()?[0].state, "unknown");
    assert!(
        !archive.claim_send(id)?,
        "uncertain attempts cannot be resent"
    );
    Ok(())
}

#[test]
fn failed_ingest_does_not_publish_metadata_and_blobs_detect_corruption() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let mut archive = Archive::open(dir.path())?;
    assert!(
        ingest(
            &mut archive,
            &blobs,
            999,
            (1, 1),
            "a@example.org",
            "me@example.com",
            10
        )
        .is_err()
    );
    assert_eq!(archive.total()?, 0);
    assert!(archive.index_work(100)?.is_empty());
    let hash = blobs.put(b"original")?;
    std::fs::write(
        dir.path()
            .join("messages")
            .join(&hash[..2])
            .join(format!("{hash}.eml")),
        b"corrupted",
    )?;
    assert!(blobs.get(&hash).is_err());
    assert!(blobs.put(b"original").is_err());
    assert!(blobs.get("../../outside").is_err());
    Ok(())
}

#[test]
fn correspondence_reserves_space_for_old_and_retired_anchors() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("me@example.com", "INBOX")?;
    let anchor = ingest(
        &mut archive,
        &blobs,
        inbox,
        (1, 1),
        "alice@example.org",
        "me@example.com",
        1,
    )?;
    let mut related = Vec::new();
    for uid in 2..=106 {
        related.push(ingest(
            &mut archive,
            &blobs,
            inbox,
            (1, uid),
            "me@example.com",
            "alice@example.org",
            i64::from(uid),
        )?);
    }
    let rows = archive.correspondence(anchor, 100)?;
    assert_eq!(rows.len(), 100);
    assert_eq!(rows.last().map(|row| row.id), Some(anchor));
    assert_eq!(rows[0].id, related[104]);
    assert_eq!(rows[98].id, related[6]);
    assert_eq!(archive.correspondence(anchor, 1)?[0].id, anchor);
    assert_eq!(archive.correspondence(anchor, 0)?.len(), 1);
    let flags: Vec<_> = (2..=106).map(|uid| (uid, false, false)).collect();
    archive.reconcile(inbox, 1, &flags)?;
    assert_eq!(
        archive.correspondence(anchor, 2)?[1].id,
        anchor,
        "retired selection remains visible"
    );
    let recipientless = ingest(
        &mut archive,
        &blobs,
        inbox,
        (1, 107),
        "alice@example.org",
        "",
        0,
    )?;
    assert_eq!(archive.correspondence(recipientless, 100)?.len(), 1);
    assert_eq!(
        archive.correspondence(recipientless, 100)?[0].id,
        recipientless
    );
    Ok(())
}
