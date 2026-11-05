use {
    mailparse::MailHeaderMap,
    moonmail_core::{BlobStore, MailRepository, MimeParser},
    moonmail_mime::StalwartParser,
    moonmail_network::{Account, ReplyMode, compose_reply, prepare_message},
    moonmail_store::{Archive, Draft, FileBlobs, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn display_names_groups_dedup_and_bcc_survive_only_in_the_frozen_envelope() -> Result {
    let account = Account::fastmail("me@example.org")?;
    let draft = Draft {
        id: "bcc-draft".into(),
        account: account.email.clone(),
        recipients: "\"Doe, Jane\" <jane@EXAMPLE.org>, Team: One <one@example.org>;".into(),
        cc: "Duplicate <jane@example.org>, André <andre@example.org>".into(),
        bcc: "Private Person <secret@example.org>, one@EXAMPLE.org".into(),
        subject: "Synthetic".into(),
        body: "Body".into(),
        ..Default::default()
    };
    let (raw, envelope) = prepare_message(&account, &draft)?;
    assert_eq!(
        envelope,
        [
            "jane@EXAMPLE.org",
            "one@example.org",
            "andre@example.org",
            "secret@example.org"
        ]
    );
    let parsed = StalwartParser::default().parse(&raw)?;
    assert_eq!(parsed.to.len(), 2);
    assert_eq!(parsed.to[0].name.as_deref(), Some("Doe, Jane"));
    assert_eq!(parsed.cc.len(), 1);
    assert_eq!(parsed.cc[0].name.as_deref(), Some("André"));
    let mime = mailparse::parse_mail(&raw)?;
    assert!(mime.headers.get_first_value("Bcc").is_none());
    let text = String::from_utf8_lossy(&raw);
    assert!(!text.contains("secret@example.org") && !text.contains("Private Person"));
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let mut archive = Archive::open(dir.path())?;
    archive.save_draft(&draft)?;
    archive.save_draft(&Draft {
        id: "unrelated".into(),
        body: "Untouched".into(),
        ..Default::default()
    })?;
    let blob = blobs.put(&raw)?;
    let id = archive.queue(&draft, &account.email, &blob, &envelope)?;
    assert!(archive.draft(&draft.id)?.is_none());
    assert!(
        archive.save_draft(&draft).is_err(),
        "stale compose events cannot resurrect queued MIME"
    );
    drop(archive);
    let archive = Archive::open(dir.path())?;
    let queued = archive
        .outbox()?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or("missing outbox")?;
    assert_eq!(queued.recipients, envelope);
    assert_eq!(blobs.get(&queued.blob)?, raw);
    assert_eq!(
        archive.draft("unrelated")?.ok_or("lost draft")?.body,
        "Untouched"
    );
    Ok(())
}

#[test]
fn bcc_only_header_injection_limits_and_account_mismatch_are_handled_explicitly() -> Result {
    let account = Account::fastmail("me@example.org")?;
    let mut draft = Draft {
        bcc: "hidden@example.org".into(),
        ..Default::default()
    };
    let (raw, envelope) = prepare_message(&account, &draft)?;
    assert_eq!(envelope, ["hidden@example.org"]);
    assert!(!String::from_utf8_lossy(&raw).contains("hidden@example.org"));
    assert!(
        mailparse::parse_mail(&raw)?
            .headers
            .get_first_value("To")
            .ok_or("missing To")?
            .contains("Undisclosed recipients")
    );
    for invalid in [
        "valid@example.org\r\nBcc: injected@example.org",
        "Name\0 <valid@example.org>",
        "not-an-address",
        "Name <é@example.org>",
    ] {
        draft.cc = invalid.into();
        assert!(prepare_message(&account, &draft).is_err(), "{invalid:?}");
    }
    draft.cc.clear();
    draft.recipients = "Peer@example.org, peer@EXAMPLE.org, peer@example.org".into();
    assert_eq!(
        prepare_message(&account, &draft)?.1,
        ["Peer@example.org", "peer@EXAMPLE.org", "hidden@example.org"]
    );
    draft.subject = "Subject\0injection".into();
    assert!(prepare_message(&account, &draft).is_err());
    draft.subject.clear();
    draft.recipients = (0..101)
        .map(|i| format!("person{i}@example.org"))
        .collect::<Vec<_>>()
        .join(",");
    assert!(prepare_message(&account, &draft).is_err());
    draft.recipients.clear();
    draft.account = "other@example.org".into();
    assert!(prepare_message(&account, &draft).is_err());
    draft.account.clear();
    draft.bcc.clear();
    assert!(prepare_message(&account, &draft).is_err());
    Ok(())
}

#[test]
fn reply_all_prefers_reply_to_excludes_self_and_quotes_without_bcc_redisclosure() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let parser = StalwartParser::default();
    let inbox = archive.mailbox("me@example.org", "INBOX")?;
    let id=archive.ingest(&parser,&blobs,IncomingMessage {
        account:"me@example.org",mailbox:inbox,origin:"incoming",epoch:1,uid:Some(1),seen:true,starred:false,received_at:1,
        raw:b"From: Author <author@example.org>\r\nReply-To: Team <team@example.org>\r\nTo: me@EXAMPLE.org, Peer <peer@example.org>\r\nCc: peer@EXAMPLE.org, Copy <copy@example.org>, me@example.org\r\nBcc: hidden@example.org\r\nSubject: Topic\r\nMessage-ID: <parent@example.org>\r\nReferences: <root@example.org>\r\n\r\nFirst line\r\nSecond line",
    })?;
    let detail = archive.detail(id)?;
    let headers = archive.reply_headers(id, &parser, &blobs)?;
    let single = compose_reply(
        &detail,
        headers.clone(),
        "me@example.org",
        ReplyMode::Sender,
    )?;
    assert_eq!(
        prepare_message(&Account::fastmail("me@example.org")?, &single)?.1,
        ["team@example.org"]
    );
    let all = compose_reply(&detail, headers, "me@example.org", ReplyMode::All)?;
    let (raw, envelope) = prepare_message(&Account::fastmail("me@example.org")?, &all)?;
    assert_eq!(
        envelope,
        ["team@example.org", "peer@example.org", "copy@example.org"]
    );
    let parsed = parser.parse(&raw)?;
    assert_eq!(parsed.to.len(), 2);
    assert_eq!(parsed.cc[0].address, "copy@example.org");
    assert_eq!(
        parsed.references,
        ["root@example.org", "parent@example.org"]
    );
    assert!(all.bcc.is_empty());
    assert!(all.body.contains("> First line\n> Second line"));
    assert!(!String::from_utf8_lossy(&raw).contains("hidden@example.org"));
    Ok(())
}

#[test]
fn replying_to_sent_mail_targets_original_recipients_and_quote_growth_is_bounded() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let parser = StalwartParser::default();
    let folder = archive.mailbox("me@example.org", "Sent")?;
    let raw = format!(
        "From: Me <me@example.org>\r\nTo: One <one@example.org>\r\nCc: Two <two@example.org>\r\nBcc: hidden@example.org\r\nSubject: Re: Topic\r\n\r\n{}",
        "é".repeat(70_000)
    );
    let id = archive.ingest(
        &parser,
        &blobs,
        IncomingMessage {
            account: "me@example.org",
            mailbox: folder,
            origin: "sent",
            epoch: 1,
            uid: Some(1),
            seen: true,
            starred: false,
            received_at: 1,
            raw: raw.as_bytes(),
        },
    )?;
    let detail = archive.detail(id)?;
    let headers = archive.reply_headers(id, &parser, &blobs)?;
    let single = compose_reply(
        &detail,
        headers.clone(),
        "me@example.org",
        ReplyMode::Sender,
    )?;
    let account = Account::fastmail("me@example.org")?;
    assert_eq!(prepare_message(&account, &single)?.1, ["one@example.org"]);
    let all = compose_reply(&detail, headers, &account.email, ReplyMode::All)?;
    assert_eq!(
        prepare_message(&account, &all)?.1,
        ["one@example.org", "two@example.org"]
    );
    assert_eq!(all.subject, "Re: Topic");
    assert!(all.body.contains("[Quoted text truncated]"));
    assert!(all.body.chars().count() < 65_000);
    Ok(())
}
