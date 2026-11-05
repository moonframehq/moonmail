//! Opt-in provider interoperability, never part of the default offline test run.
//! Uses dedicated consumer Gmail accounts and disposable local archives, not Keychain.
mod support {
    pub mod gmail;
}

use {
    moonmail_core::{BlobStore, MailRepository, MailboxRole, MimeAttachments, MimeParser},
    moonmail_mime::{MailparseParser, StalwartParser},
    moonmail_network::{ReplyMode, compose_reply},
    moonmail_store::{Archive, Draft, DraftAttachment, FileBlobs},
    std::time::{Duration, Instant},
    support::gmail::{Peer, Result, check, stage},
};

#[test]
#[ignore = "sends real email; requires MOONMAIL_LIVE_GMAIL=1 and two dedicated Gmail accounts"]
fn gmail_round_trip() -> Result {
    check(
        std::env::var("MOONMAIL_LIVE_GMAIL").as_deref() == Ok("1"),
        "Set MOONMAIL_LIVE_GMAIL=1 to run the live test",
    )?;
    let a = Peer::load("GMAIL_TEST_A_EMAIL", "GMAIL_TEST_A_APP_PASSWORD")?;
    let b = Peer::load("GMAIL_TEST_B_EMAIL", "GMAIL_TEST_B_APP_PASSWORD")?;
    support::gmail::distinct_accounts(&a.account.email, &b.account.email)?;
    // Validate both sides before the first submission. Never retry SMTP.
    a.check_connection()?;
    b.check_connection()?;

    let run = uuid::Uuid::new_v4().to_string();
    println!("Gmail live run: {run}");
    let deadline = Instant::now() + Duration::from_secs(8 * 60);
    let attachment_bytes = format!("Moonmail synthetic attachment {run}\n").into_bytes();
    let attachment_blobs = FileBlobs::attachments(a.root.path());
    let draft = Draft {
        id: uuid::Uuid::new_v4().to_string(),
        account: a.account.email.clone(),
        recipients: b.account.email.clone(),
        subject: format!("[Moonmail CI] {run}"),
        body: format!("Synthetic Gmail round-trip test.\nRun: {run}\nUnicode: café 🌙\n"),
        attachments: vec![DraftAttachment {
            id: "synthetic-file".into(),
            name: "moonmail-ci.txt".into(),
            blob: stage(
                "store synthetic attachment",
                attachment_blobs.put(&attachment_bytes),
            )?,
            bytes: attachment_bytes.len() as u64,
        }],
        ..Default::default()
    };
    let original = a.send(&draft, &b)?;
    println!("A → B accepted by SMTP; waiting for Inbox delivery");
    let received = b.wait_for(&original, MailboxRole::Inbox, deadline)?;
    let archive = stage("open B archive", Archive::open(b.root.path()))?;
    let blobs = FileBlobs::new(b.root.path());
    let raw = stage(
        "read received message",
        blobs.get(&stage("received blob", archive.message_blob(received))?),
    )?;
    let attachments = stage(
        "parse received attachments",
        MailparseParser.attachments(&raw),
    )?;
    check(attachments.len() == 1, "Expected one received attachment")?;
    check(
        attachments[0].name == "moonmail-ci.txt",
        "Attachment name changed",
    )?;
    check(
        stage(
            "decode received attachment",
            MailparseParser.attachment(&raw, &attachments[0].id),
        )? == attachment_bytes,
        "Attachment content changed during delivery",
    )?;

    // Construct the actual application's reply from the downloaded message.
    let parser = StalwartParser::default();
    let detail = stage("read reply source", archive.detail(received))?;
    let headers = stage(
        "read reply headers",
        archive.reply_headers(received, &parser, &blobs),
    )?;
    let mut reply = stage(
        "compose reply",
        compose_reply(&detail, headers, &b.account.email, ReplyMode::Sender),
    )?;
    reply.id = uuid::Uuid::new_v4().to_string();
    reply.body = format!("Synthetic reply for {run}.\n{}", reply.body);
    drop(archive);
    let response = b.send(&reply, &a)?;
    check(
        response.parsed.in_reply_to == original.parsed.message_id,
        "Reply lost In-Reply-To",
    )?;
    check(
        response.parsed.references.contains(&original.rfc_id),
        "Reply lost References",
    )?;
    println!("B → A reply accepted by SMTP; waiting for return delivery");
    a.wait_for(&response, MailboxRole::Inbox, deadline)?;
    a.wait_for(&original, MailboxRole::Sent, deadline)?;
    b.wait_for(&response, MailboxRole::Sent, deadline)?;

    // Reopen both archives and reconnect again, then observe unique copies.
    a.sync(deadline)?;
    b.sync(deadline)?;
    for (peer, message, role) in [
        (&a, &response, MailboxRole::Inbox),
        (&b, &original, MailboxRole::Inbox),
        (&a, &original, MailboxRole::Sent),
        (&b, &response, MailboxRole::Sent),
    ] {
        let id = peer
            .find(message, role)?
            .ok_or("Message missing after reconnect")?;
        peer.verify(id, message)?;
    }
    for (peer, sent) in [(&a, &original), (&b, &response)] {
        let archive = stage("reopen outbox", Archive::open(peer.root.path()))?;
        let item = stage(
            "read accepted outbox item",
            archive.outbox_item(sent.outbox_id),
        )?;
        check(
            item.state == "sent" && item.sent_status == "visible",
            "Outbox did not reconcile its unique Sent copy",
        )?;
        check(
            item.sent_copy_state == "server",
            "Gmail unexpectedly requested an IMAP Sent append",
        )?;
    }
    println!(
        "PASS: two-way delivery, MIME/body/attachment, reply ancestry, Sent copies, and reconnect ({run})"
    );
    Ok(())
}

#[test]
fn live_accounts_must_be_distinct_gmail_mailboxes_not_aliases() -> Result {
    use support::gmail::distinct_accounts;
    distinct_accounts("synthetic-a@gmail.com", "synthetic-b@gmail.com")?;
    check(
        distinct_accounts("first.last@gmail.com", "firstlast@googlemail.com").is_err(),
        "Dot/domain aliases accepted",
    )?;
    check(
        distinct_accounts("first@gmail.com", "first+test@gmail.com").is_err(),
        "Plus alias accepted",
    )?;
    check(
        distinct_accounts("first@gmail.com", "second@example.org").is_err(),
        "Non-Gmail account accepted",
    )?;
    Ok(())
}

#[test]
fn received_message_validation_rejects_wrong_sender_body_and_reply_ancestry() -> Result {
    let parser = StalwartParser::default();
    let original = stage("parse synthetic fixture", parser.parse(b"From: a@example.org\r\nTo: b@example.org\r\nSubject: Synthetic\r\nMessage-ID: <test@example.org>\r\n\r\nExpected body"))?;
    support::gmail::verify_parsed(&original, &original)?;
    for field in ["sender", "body", "reply"] {
        let mut altered = original.clone();
        match field {
            "sender" => altered.from.clear(),
            "body" => altered.text_body = Some("different".into()),
            _ => altered.in_reply_to = Some("unrelated@example.org".into()),
        }
        check(
            support::gmail::verify_parsed(&altered, &original).is_err(),
            "Altered received message passed validation",
        )?;
    }
    Ok(())
}
