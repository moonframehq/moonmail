use {
    moonmail_core::{BlobStore, MailRepository, MimeParser},
    moonmail_mime::StalwartParser,
    moonmail_network::{Account, prepare_message},
    moonmail_store::{Archive, Draft, FileBlobs, IncomingMessage},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn reply_to_reply_survives_draft_reopen_and_stays_in_the_root_thread() -> Result {
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let parser = StalwartParser::default();
    let mut archive = Archive::open(dir.path())?;
    let account = Account::fastmail("me@example.org")?;
    let inbox = archive.mailbox(&account.email, "INBOX")?;
    let mut root = None;
    let mut parent = None;
    for (uid, headers) in [
        (1, "Message-ID: <root@example.org>\r\n"),
        (
            2,
            "Message-ID: <parent@example.org>\r\nReferences: <root@example.org>\r\nIn-Reply-To: <root@example.org>\r\n",
        ),
    ] {
        let raw = format!(
            "From: sender@example.org\r\nTo: me@example.org\r\nSubject: Chain\r\n{headers}\r\nSynthetic body"
        );
        let id = archive.ingest(
            &parser,
            &blobs,
            IncomingMessage {
                account: &account.email,
                mailbox: inbox,
                origin: &uid.to_string(),
                epoch: 1,
                uid: Some(uid),
                seen: false,
                starred: false,
                received_at: i64::from(uid),
                raw: raw.as_bytes(),
            },
        )?;
        if uid == 1 {
            root = Some(id);
        } else {
            parent = Some(id);
        }
    }
    let headers = archive.reply_headers(parent.ok_or("missing parent")?, &parser, &blobs)?;
    let draft = Draft {
        id: "synthetic-draft".into(),
        recipients: "sender@example.org".into(),
        subject: "Re: Chain".into(),
        body: "Synthetic reply".into(),
        reply_id: headers.message_id.ok_or("missing RFC ID")?,
        references: headers.references,
        ..Default::default()
    };
    archive.save_draft(&draft)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    let draft = archive.latest_draft()?.ok_or("missing draft")?;
    assert_eq!(draft.references, ["root@example.org"]);
    let (raw, recipients) = prepare_message(&account, &draft)?;
    assert_eq!(recipients, ["sender@example.org"]);
    let parsed = parser.parse(&raw)?;
    assert_eq!(parsed.in_reply_to.as_deref(), Some("parent@example.org"));
    assert_eq!(
        parsed.references,
        ["root@example.org", "parent@example.org"]
    );
    let sent = archive.mailbox(&account.email, "Sent")?;
    let reply = archive.ingest(
        &parser,
        &blobs,
        IncomingMessage {
            account: &account.email,
            mailbox: sent,
            origin: "reply",
            epoch: 1,
            uid: Some(3),
            seen: true,
            starred: false,
            received_at: 3,
            raw: &raw,
        },
    )?;
    let thread = archive.thread(root.ok_or("missing root")?, 10)?;
    assert_eq!(thread.len(), 3);
    assert!(thread.iter().any(|message| message.id == reply));
    let hash = blobs.put(&raw)?;
    archive.queue(&draft, &account.email, &hash, &recipients)?;
    assert_eq!(
        blobs.get(&archive.outbox()?[0].blob)?,
        raw,
        "queued MIME stays frozen"
    );
    Ok(())
}

#[test]
fn outgoing_reference_chain_is_bounded_deduplicated_and_cannot_inject_headers() -> Result {
    let account = Account::fastmail("me@example.org")?;
    let mut draft = Draft {
        recipients: "sender@example.org".into(),
        reply_id: "parent@example.org".into(),
        references: (0..150).map(|id| format!("{id}@example.org")).collect(),
        ..Default::default()
    };
    draft.references.extend([
        "149@example.org".into(),
        "parent@example.org".into(),
        "bad\r\nBcc: leaked@example.org".into(),
    ]);
    let (raw, _) = prepare_message(&account, &draft)?;
    let parsed = StalwartParser::default().parse(&raw)?;
    assert_eq!(parsed.references.len(), 100);
    assert_eq!(parsed.references[0], "0@example.org");
    assert_eq!(parsed.references[1], "52@example.org");
    assert_eq!(
        parsed.references.last().map(String::as_str),
        Some("parent@example.org")
    );
    assert!(!String::from_utf8(raw)?.contains("leaked"));
    draft.reply_id = "parent@example.org\r\nBcc: injected@example.org".into();
    assert!(prepare_message(&account, &draft).is_err());
    draft.reply_id.clear();
    draft.references.clear();
    let parsed = StalwartParser::default().parse(&prepare_message(&account, &draft)?.0)?;
    assert!(parsed.references.is_empty());
    assert!(parsed.in_reply_to.is_none());
    Ok(())
}
