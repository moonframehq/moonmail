use {
    mailparse::MailHeaderMap,
    moonmail_core::{BlobStore, MailRepository, MimeParser},
    moonmail_mime::StalwartParser,
    moonmail_network::{
        Account, ReplyMode, choose_identity, compose_reply_with_identities, insert_signature,
        prepare_message,
    },
    moonmail_store::{Archive, Draft, FileBlobs, Identity, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn identity() -> Identity {
    Identity {
        id: "work".into(),
        account: "me@example.org".into(),
        name: "Élodie, Work".into(),
        address: "work@example.org".into(),
        reply_to: "replies@example.org".into(),
        signature: "Élodie\nDesign team".into(),
        is_default: true,
    }
}

#[test]
fn identity_snapshot_survives_edit_delete_restore_and_uses_alias_envelope_with_one_from_header()
-> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let account = Account::fastmail("me@example.org")?;
    let identity = identity();
    archive.save_identity(&identity)?;
    let mut draft = Draft {
        id: "draft".into(),
        account: account.email.clone(),
        identity: Some(identity.clone()),
        recipients: "peer@example.org".into(),
        body: "My edited text".into(),
        ..Default::default()
    };
    insert_signature(&mut draft);
    archive.save_draft(&draft)?;
    let mut changed = identity.clone();
    changed.name = "Changed".into();
    changed.signature = "New signature".into();
    archive.save_identity(&changed)?;
    archive.delete_identity(&account.email, &identity.id)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    let saved = archive.draft(&draft.id)?.ok_or("missing draft")?;
    assert_eq!(saved.body, draft.body);
    assert_eq!(
        saved.identity.as_ref().ok_or("missing identity")?.name,
        identity.name
    );
    let (raw, envelope) = prepare_message(&account, &saved)?;
    let headers = mailparse::parse_mail(&raw)?;
    assert_eq!(headers.headers.get_all_values("From").len(), 1);
    let parsed = StalwartParser::default().parse(&raw)?;
    assert_eq!(parsed.reply_to[0].address, "replies@example.org");
    assert_eq!(parsed.from[0].address, "work@example.org");
    assert_eq!(parsed.from[0].name.as_deref(), Some("Élodie, Work"));
    let blobs = FileBlobs::new(dir.path());
    let id = archive.queue_with_message_id(
        &saved,
        &account.email,
        &blobs.put(&raw)?,
        &envelope,
        parsed.message_id.as_deref(),
    )?;
    assert_eq!(archive.outbox_item(id)?.sender, "work@example.org");
    assert_eq!(archive.outbox_item(id)?.account, "me@example.org");
    let restored = archive.restore_outbox_draft(id, "restored")?;
    assert_eq!(restored.sender(), "work@example.org");
    assert_eq!(restored.body, draft.body);
    let mut invalid = restored.clone();
    invalid.account = "other@example.org".into();
    assert!(archive.save_draft(&invalid).is_err());
    assert!(prepare_message(&Account::fastmail("other@example.org")?, &invalid).is_err());
    invalid = restored;
    invalid
        .identity
        .as_mut()
        .ok_or("missing identity")?
        .reply_to = "good@example.org\r\nBcc: injected@example.org".into();
    assert!(prepare_message(&account, &invalid).is_err());
    Ok(())
}

#[test]
fn replies_choose_explicit_addressed_identity_and_exclude_only_owned_aliases() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let inbox = archive.mailbox("me@example.org", "INBOX")?;
    let identities = vec![identity()];
    let raw = b"From: peer@example.org\r\nTo: work@EXAMPLE.org, Work@example.org, me@example.org\r\nCc: work+unknown@example.org, copy@example.org\r\nBcc: hidden@example.org\r\nMessage-ID: <original@example.org>\r\nSubject: Identity test\r\n\r\nOriginal text";
    let id = archive.ingest(
        &StalwartParser::default(),
        &blobs,
        IncomingMessage {
            account: "me@example.org",
            mailbox: inbox,
            origin: "original",
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 0,
            raw,
        },
    )?;
    let headers = archive.reply_headers(id, &StalwartParser::default(), &blobs)?;
    assert_eq!(
        choose_identity(&identities, "me@example.org", Some(&headers))
            .ok_or("missing chosen identity")?
            .id,
        "work"
    );
    assert!(choose_identity(&identities, "other@example.org", Some(&headers)).is_none());
    let reply = compose_reply_with_identities(
        &archive.detail(id)?,
        headers.clone(),
        "me@example.org",
        ReplyMode::All,
        &identities,
    )?;
    assert_eq!(reply.recipients, "peer@example.org, Work@example.org");
    assert_eq!(reply.cc, "work+unknown@example.org, copy@example.org");
    assert!(reply.bcc.is_empty());
    let mut own = headers;
    own.addresses.from[0].address = "work@EXAMPLE.org".into();
    own.addresses.reply_to = vec![moonmail_core::MailboxAddress {
        name: None,
        address: "replies@example.org".into(),
    }];
    own.addresses.to[0].address = "peer@example.org".into();
    let reply = compose_reply_with_identities(
        &archive.detail(id)?,
        own,
        "me@example.org",
        ReplyMode::Sender,
        &identities,
    )?;
    assert!(reply.recipients.starts_with("peer@example.org"));
    assert!(!reply.recipients.contains("work@EXAMPLE.org"));
    Ok(())
}
