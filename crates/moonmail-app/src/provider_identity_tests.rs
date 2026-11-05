use super::{tests::data, *};

#[test]
fn account_selection_preserves_content_and_freezes_the_selected_sender() -> Result<()> {
    let dir = tempfile::tempdir()?;
    for email in ["you@example.com", "work@example.org"] {
        Account::fastmail(email)?.save(dir.path())?;
    }
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    let mut identity = moonmail_store::Identity {
        id: "work-alias".into(),
        account: "work@example.org".into(),
        name: "Work team".into(),
        address: "team@example.org".into(),
        reply_to: "replies@example.org".into(),
        signature: "Configured signature".into(),
        ..Default::default()
    };
    mail.archive_mut()?.save_identity(&identity)?;
    mail.handle(data(json!({"action":"compose"})))?;
    let mut original = mail.draft.clone().context("missing draft")?;
    original.recipients = "peer@example.org".into();
    original.cc = "copy@example.org".into();
    original.bcc = "hidden@example.org".into();
    original.subject = "Account selection".into();
    original.body = "Hand-edited body and signature".into();
    original.reply_id = "parent@example.org".into();
    original.references = vec!["ancestor@example.org".into()];
    mail.archive()?.save_draft(&original)?;
    mail.handle(data(json!({"action":"draft_open","id":original.id})))?;
    let file = dir.path().join("synthetic.txt");
    std::fs::write(&file, b"Synthetic attachment")?;
    mail.handle(data(
        json!({"action":"draft_attach","id":original.id,"paths":[file]}),
    ))?;
    original = mail.draft.clone().context("missing attached draft")?;
    for event in [
        json!({"action":"draft_account","id":"stale","account":"work@example.org"}),
        json!({"action":"draft_account","id":original.id,"account":"missing@example.org"}),
        json!({"action":"draft_account","id":original.id,"account":"you@example.com","identity":identity.id}),
    ] {
        assert!(mail.handle(data(event)).is_err());
        assert_eq!(serde_json::to_value(&mail.draft)?, json!(original));
        assert_eq!(
            serde_json::to_value(mail.archive()?.draft(&original.id)?)?,
            json!(original)
        );
    }
    mail.handle(data(json!({"action":"draft_account","id":original.id,"account":identity.account,"identity":identity.id})))?;
    let mut expected = original.clone();
    expected.account = identity.account.clone();
    expected.identity = Some(identity.clone());
    assert_eq!(serde_json::to_value(&mail.draft)?, json!(expected));
    assert!(
        mail.snapshot()?["from_accounts"]
            .as_array()
            .context("choices")?
            .iter()
            .any(|choice| choice["identity"] == identity.id && choice["current"] == true)
    );
    // Editing or deleting configuration never changes the frozen draft sender,
    // and the menu must not check a row that now represents a different sender.
    identity.address = "changed@example.org".into();
    mail.archive_mut()?.save_identity(&identity)?;
    assert!(
        mail.snapshot()?["from_accounts"]
            .as_array()
            .context("choices")?
            .iter()
            .all(|choice| choice["current"] == false)
    );
    mail.archive()?
        .delete_identity(&identity.account, &identity.id)?;
    drop(mail);
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    mail.handle(data(json!({"action":"draft_open","id":expected.id})))?;
    assert_eq!(serde_json::to_value(&mail.draft)?, json!(expected));
    Account::remove(dir.path(), &expected.account)?;
    assert!(
        mail.handle(data(json!({"action":"queue_only","id":expected.id})))
            .is_err()
    );
    assert!(mail.archive()?.outbox()?.is_empty());
    Account::fastmail(&expected.account)?.save(dir.path())?;
    mail.handle(data(json!({"action":"queue_only","id":expected.id})))?;
    let item = mail.archive()?.outbox()?.remove(0);
    assert_eq!(item.account, "work@example.org");
    assert_eq!(item.sender, "team@example.org");
    let raw = FileBlobs::new(dir.path()).get(&item.blob)?;
    let mime = String::from_utf8(raw)?;
    assert!(
        mime.lines()
            .any(|line| line.starts_with("From:") && line.contains("team@example.org"))
    );
    assert!(
        mime.lines()
            .any(|line| line.starts_with("Reply-To:") && line.contains("replies@example.org"))
    );
    Ok(())
}

#[test]
fn legacy_blank_account_requires_an_explicit_from_selection() -> Result<()> {
    let dir = tempfile::tempdir()?;
    Account::fastmail("you@example.com")?.save(dir.path())?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    let draft = Draft {
        id: "legacy".into(),
        recipients: "peer@example.org".into(),
        ..Default::default()
    };
    mail.archive()?.save_draft(&draft)?;
    mail.handle(data(json!({"action":"draft_open","id":draft.id})))?;
    assert!(
        mail.handle(data(json!({"action":"queue_only","id":draft.id})))
            .is_err()
    );
    assert!(mail.archive()?.outbox()?.is_empty());
    mail.handle(data(
        json!({"action":"draft_account","id":draft.id,"account":"you@example.com"}),
    ))?;
    mail.handle(data(json!({"action":"queue_only","id":draft.id})))?;
    assert_eq!(mail.archive()?.outbox()?[0].account, "you@example.com");
    Ok(())
}

#[test]
fn identity_events_are_account_and_draft_bound_and_signatures_do_not_rewrite_edits() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    mail.initialize()?;
    let identity = mail.identity_new_id.clone();
    let event = json!({"action":"identity_save","id":identity,"account":"you@example.com","name":"You at work","address":"work@example.org","reply_to":"reply@example.org","signature":"You\nWork team"});
    mail.handle(data(event.clone()))?;
    mail.handle(data(event))?;
    assert_eq!(mail.archive()?.identities("you@example.com")?.len(), 1);
    assert_ne!(mail.identity_new_id, identity);
    mail.handle(data(
        json!({"action":"identity_default","id":identity,"account":"you@example.com"}),
    ))?;
    mail.handle(data(json!({"action":"compose"})))?;
    let draft = mail.draft.clone().context("missing draft")?;
    assert_eq!(draft.sender(), "work@example.org");
    assert_eq!(draft.body.matches("Work team").count(), 1);
    mail.handle(data(json!({"action":"draft","id":draft.id,"field":"body","value":"Hand-edited signature and text"})))?;
    mail.handle(data(json!({"action":"draft_open","id":draft.id})))?;
    mail.handle(data(
        json!({"action":"draft_identity","id":draft.id,"identity":""}),
    ))?;
    assert_eq!(
        mail.draft.as_ref().context("missing draft")?.body,
        "Hand-edited signature and text"
    );
    assert!(
        mail.handle(data(
            json!({"action":"draft_identity","id":"stale","identity":identity})
        ))
        .is_err()
    );
    mail.handle(data(
        json!({"action":"draft_identity","id":draft.id,"identity":identity}),
    ))?;
    mail.handle(data(json!({"action":"draft_signature","id":draft.id})))?;
    assert!(
        mail.draft
            .as_ref()
            .context("missing draft")?
            .body
            .ends_with("Hand-edited signature and text")
    );
    assert!(
        mail.handle(data(
            json!({"action":"identity_delete","id":identity,"account":"other@example.org"})
        ))
        .is_err()
    );
    mail.handle(data(
        json!({"action":"identity_delete","id":identity,"account":"you@example.com"}),
    ))?;
    mail.handle(data(
        json!({"action":"draft","id":draft.id,"field":"recipients","value":"peer@example.org"}),
    ))?;
    mail.handle(data(json!({"action":"queue_only","id":draft.id})))?;
    let item = mail.archive()?.outbox()?.remove(0);
    assert_eq!(item.sender, "work@example.org");
    assert_eq!(item.sent_status, "pending");
    Ok(())
}
