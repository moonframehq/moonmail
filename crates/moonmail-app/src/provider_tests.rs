use {
    super::*,
    moonframe::moonframe_core::{AppLayer, Roots, Workspace},
    serde_json::Value,
};

pub(super) fn data(value: Value) -> Data {
    match value {
        Value::Null => Data::Nil,
        Value::Bool(value) => Data::Bool(value),
        Value::Number(value) => Data::Integer(value.as_i64().unwrap_or_default()),
        Value::String(value) => Data::String(value),
        Value::Array(values) => Data::List(values.into_iter().map(data).collect()),
        Value::Object(values) => {
            Data::map(values.into_iter().map(|(key, value)| (key, data(value))))
        },
    }
}

#[test]
fn populated_ui_actions_reader_search_draft_and_all_panes() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail =
        Mail::new(dir.path().to_owned(), true).with_font_config(dir.path().join("ui/style.luau"));
    let mut archive = Archive::open(dir.path())?;
    moonmail_store::seed_demo(&mut archive, &FileBlobs::new(dir.path()))?;
    let mut index = crate::local::open_index(dir.path(), &archive)?;
    while crate::local::index_batch(&archive, &mut index)? {}
    mail.index = Some(index);
    mail.archive = Some(archive);
    mail.reader_dir = Some(tempfile::tempdir_in(dir.path())?);
    mail.refresh()?;
    let id = mail.page.rows[0].id;
    mail.select(id, true)?;
    // Use the same toolbox components as the running app, located from Cargo
    // metadata rather than a machine-specific checkout path or copied fixtures.
    let metadata = std::process::Command::new("cargo")
        .args(["metadata", "--locked", "--format-version=1"])
        .output()?;
    assert!(metadata.status.success());
    let metadata: Value = serde_json::from_slice(&metadata.stdout)?;
    let manifest = metadata["packages"]
        .as_array()
        .context("No packages")?
        .iter()
        .find(|p| p["name"] == "moonframe")
        .and_then(|p| p["manifest_path"].as_str())
        .context("No Moonframe package")?;
    let toolbox = PathBuf::from(manifest)
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .context("No toolbox root")?
        .join("app");
    let mut workspace = Workspace::open(
        Roots::bundled(
            toolbox,
            Vec::new(),
            AppLayer {
                path: PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/app")),
                keep: vec!["feedback".into()],
            },
        ),
        false,
    );
    workspace.set_host("mail", data(mail.snapshot()?));
    assert!(
        workspace.problems().is_empty(),
        "{:?}",
        workspace.problems()
    );
    mail.handle(data(json!({"action":"threaded"})))?;
    assert_eq!(mail.page.rows.len(), 24);
    let root = mail.page.rows[0].id;
    mail.handle(data(json!({"action":"expand","id":root})))?;
    assert_eq!(mail.listing()?.len(), 26);
    let child = mail.listing()?[1]["id"]
        .as_u64()
        .context("No expanded child")?;
    mail.handle(data(json!({"action":"select","id":child})))?;
    assert_eq!(
        mail.selected.as_ref().context("No selection")?.summary.id.0,
        child
    );
    assert_eq!(mail.listing()?.len(), 26);
    workspace.set_host("mail", data(mail.snapshot()?));
    assert!(
        workspace.problems().is_empty(),
        "{:?}",
        workspace.problems()
    );
    mail.handle(data(json!({"action":"expand","id":root})))?;
    assert_eq!(mail.listing()?.len(), 24);
    mail.handle(data(json!({"action":"sort","field":"sender"})))?;
    assert!(mail.query.ascending);
    mail.handle(data(json!({"action":"settings_save","interface_family":".AppleSystemUIFont","list_family":"Menlo","font_size":"14","reader_family":"Helvetica","reader_size":"15","threaded":false,"mark_read":false})))?;
    assert_eq!(
        crate::fonts::Preferences::load(dir.path())?.list_family,
        "Menlo"
    );
    // Without a saved choice the wax seal is the icon; a save may pick
    // another, a client that omits the field keeps it, and an unknown name
    // is refused without changing it.
    use crate::app_icon::AppIcon;
    assert_eq!(
        crate::fonts::Preferences::load(dir.path())?.app_icon,
        AppIcon::WaxSeal
    );
    let save = |icon: Option<&str>| {
        let mut request = json!({"action":"settings_save","interface_family":".AppleSystemUIFont","list_family":"Menlo","font_size":"14","reader_family":"Helvetica","reader_size":"15","threaded":false,"mark_read":false});
        if let Some(icon) = icon {
            request["app_icon"] = json!(icon);
        }
        data(request)
    };
    mail.handle(save(Some("orbit")))?;
    assert_eq!(
        crate::fonts::Preferences::load(dir.path())?.app_icon,
        AppIcon::Orbit
    );
    mail.handle(save(None))?;
    assert_eq!(
        crate::fonts::Preferences::load(dir.path())?.app_icon,
        AppIcon::Orbit
    );
    assert!(mail.handle(save(Some("sunset"))).is_err());
    assert_eq!(
        crate::fonts::Preferences::load(dir.path())?.app_icon,
        AppIcon::Orbit
    );
    assert_eq!(mail.page.rows.len(), 72);
    let unread = mail
        .page
        .rows
        .iter()
        .find(|row| row.unread)
        .context("No unread message")?
        .id;
    mail.select(unread, true)?;
    assert!(mail.archive()?.detail(unread)?.summary.unread);
    workspace.set_host("mail", data(mail.snapshot()?));
    mail.handle(data(json!({"action":"fonts_load"})))?;
    assert!(mail.font_families.contains(&".AppleSystemUIFont".into()));
    mail.handle(data(
        json!({"action":"font_preview","family":"Menlo","size":"14"}),
    ))?;
    assert!(mail.font_preview_url.contains("font-"));
    // Previewing is transient and must not change saved preferences.
    assert_eq!(crate::fonts::Preferences::load(dir.path())?.reader.size, 15);
    workspace.set_host("mail", data(mail.snapshot()?));
    for page in [
        "appearance",
        "fonts",
        "reading",
        "sync",
        "account",
        "storage",
    ] {
        workspace.send("shell", "settings", data(json!({"page":page})));
        assert!(
            workspace.problems().is_empty(),
            "{page}: {:?}",
            workspace.problems()
        );
    }
    workspace.send("shell", "settings", data(json!({"page":"account"})));
    for message in [
        json!("add-account"),
        json!("provider-list"),
        json!({"provider":"google"}),
        json!({"provider":"fastmail"}),
        json!({"msg":"email","value":"synthetic@fastmail.com"}),
        json!("cancel-account"),
    ] {
        workspace.send("shell", "settings", data(message));
        assert!(
            workspace.problems().is_empty(),
            "{:?}",
            workspace.problems()
        );
    }
    workspace.take_outbox();
    workspace.send("shell", "settings", "add-account".into());
    workspace.send("shell", "settings", data(json!({"provider":"google"})));
    workspace.send(
        "shell",
        "settings",
        data(json!({"msg":"email","value":"synthetic@gmail.com"})),
    );
    workspace.send("shell", "settings", "connect-account".into());
    let google = workspace.take_outbox();
    assert_eq!(google.len(), 1);
    assert_eq!(
        google[0].2.get("provider").and_then(Data::as_str),
        Some("google")
    );
    assert_eq!(
        google[0]
            .2
            .get("config")
            .and_then(|config| config.get("smtp_host"))
            .and_then(Data::as_str),
        Some("smtp.gmail.com")
    );
    workspace.send("shell", "settings", "cancel-account".into());
    workspace.send("shell", "settings", "add-account".into());
    workspace.send("shell", "settings", data(json!({"provider":"fastmail"})));
    workspace.send(
        "shell",
        "settings",
        data(json!({"msg":"email","value":" synthetic@custom.test "})),
    );
    workspace.send("shell", "settings", "connect-account".into());
    let outbox = workspace.take_outbox();
    assert_eq!(
        outbox.len(),
        1,
        "manual provider choice must survive entering the email"
    );
    assert_eq!(
        outbox[0].2.get("action").and_then(Data::as_str),
        Some("login")
    );
    assert_eq!(
        outbox[0].2.get("email").and_then(Data::as_str),
        Some("synthetic@custom.test")
    );
    workspace.send("shell", "settings", "cancel-account".into());
    mail.handle(data(json!({"action":"search","value":"quieter"})))?;
    assert!(!mail.page.rows.is_empty());
    assert!(
        mail.page
            .rows
            .iter()
            .all(|row| row.subject.contains("quieter"))
    );
    mail.handle(data(json!({"action":"reply"})))?;
    let draft = mail.draft.as_ref().context("No reply draft")?;
    assert!(draft.subject.starts_with("Re:"));
    let draft_id = draft.id.clone();
    mail.handle(data(
        json!({"action":"draft","id":draft_id,"field":"body","value":"Synthetic reply"}),
    ))?;
    assert_eq!(
        mail.archive()?
            .latest_draft()?
            .context("No saved draft")?
            .body,
        "Synthetic reply"
    );
    let attached_file = dir.path().join("synthetic.txt");
    std::fs::write(&attached_file, b"Synthetic attachment")?;
    mail.handle(data(
        json!({"action":"draft_attach","id":draft_id,"paths":[attached_file]}),
    ))?;
    let outbox_draft = Draft {
        id: "outbox-layout".into(),
        subject: "Synthetic queued message".into(),
        recipients: "peer@example.org".into(),
        ..Default::default()
    };
    let blob = FileBlobs::new(dir.path()).put(b"From: you@example.com\r\n\r\nSynthetic")?;
    mail.archive_mut()?.queue(
        &outbox_draft,
        "you@example.com",
        &blob,
        &["peer@example.org".into()],
    )?;
    workspace.set_host("mail", data(mail.snapshot()?));
    for pane in [
        "compose",
        "drafts",
        "settings",
        "outbox",
        "organize",
        "operations",
        "mail",
        "rich",
        "thread",
        "links",
        "searches",
        "mailbox-chooser",
        "move-chooser",
        "copy-chooser",
        "folders",
        "tags",
        "rules",
        "export-selected",
        "identities",
    ] {
        workspace.send("shell", "shell", pane.into());
        assert!(
            workspace.problems().is_empty(),
            "{pane}: {:?}",
            workspace.problems()
        );
    }
    // A composer requests a separate native window. Opening it leaves the
    // mailbox view in place, and mailbox navigation cannot retarget its draft.
    workspace.send("shell", "shell", "mail".into());
    let mailbox_view = format!("{:?}", workspace.slot("shell").context("No shell")?.view());
    workspace.take_requests();
    workspace.send("provider:mail", "mail-composer", "open".into());
    assert!(
        workspace
            .take_requests()
            .iter()
            .any(|(_, request)| matches!(
                request,
                moonframe::moonframe_core::Request::Window(
                    moonframe::moonframe_core::WindowRequest::Open { name, .. }
                ) if name == "mail-composer"
            ))
    );
    assert_eq!(
        mailbox_view,
        format!("{:?}", workspace.slot("shell").context("No shell")?.view())
    );
    for toggle in ["toggle-cc", "toggle-from", "toggle-send"] {
        workspace.send("mail-composer", "mail-composer", toggle.into());
        assert!(
            workspace.problems().is_empty(),
            "{:?}",
            workspace.problems()
        );
    }
    workspace.send("mail-composer", "mail-composer", "close-from".into());
    Account::fastmail("you@example.com")?.save(dir.path())?;
    Account::fastmail("work@example.org")?.save(dir.path())?;
    workspace.set_host("mail", data(mail.snapshot()?));
    workspace.take_outbox();
    workspace.send("mail-composer", "mail-composer", "toggle-from".into());
    workspace.send(
        "mail-composer",
        "mail-composer",
        data(json!({"from_step":1,"id":draft_id})),
    );
    workspace.send(
        "mail-composer",
        "mail-composer",
        data(json!({"from_choose":true,"id":draft_id})),
    );
    let events = workspace.take_outbox();
    let selection = events
        .iter()
        .find(|event| event.2.get("action").and_then(Data::as_str) == Some("draft_account"))
        .context("Missing From account selection")?;
    assert_eq!(
        selection.2.get("account").and_then(Data::as_str),
        Some("work@example.org")
    );
    assert_eq!(
        selection.2.get("id").and_then(Data::as_str),
        Some(draft_id.as_str())
    );
    // Escape and a changed registry both invalidate queued menu key events.
    for change_accounts in [false, true] {
        workspace.send("mail-composer", "mail-composer", "toggle-from".into());
        if change_accounts {
            Account::remove(dir.path(), "work@example.org")?;
            workspace.set_host("mail", data(mail.snapshot()?));
        } else {
            workspace.send("mail-composer", "mail-composer", "close-from".into());
        }
        workspace.send(
            "mail-composer",
            "mail-composer",
            data(json!({"from_choose":true,"id":draft_id})),
        );
        assert!(workspace.take_outbox().is_empty());
    }
    workspace.take_requests();
    workspace.send("mail-composer", "mail-composer", "toggle-from".into());
    workspace.send(
        "mail-composer",
        "mail-composer",
        data(json!({"from_step":-1,"id":draft_id})),
    );
    workspace.send(
        "mail-composer",
        "mail-composer",
        data(json!({"from_choose":true,"id":draft_id})),
    );
    assert!(workspace.take_requests().iter().any(|(_, request)| matches!(request,
        moonframe::moonframe_core::Request::Window(moonframe::moonframe_core::WindowRequest::Open { name, .. }) if name == "settings")));
    assert!(
        workspace.problems().is_empty(),
        "{:?}",
        workspace.problems()
    );
    let saved = mail.draft.clone().context("No composer draft")?;
    mail.handle(data(json!({"action":"move","value":"down"})))?;
    assert_eq!(
        mail.draft.as_ref().context("No draft after navigation")?.id,
        saved.id
    );
    mail.handle(data(json!({"action":"draft","id":saved.id,"field":"body","value":"Written while reading another message"})))?;
    mail.handle(data(json!({"action":"draft_close","id":saved.id})))?;
    assert_eq!(
        mail.archive()?
            .draft(&saved.id)?
            .context("Close lost draft")?
            .body,
        "Written while reading another message"
    );
    assert!(
        mail.handle(data(json!({"action":"draft_close","id":"stale-window"})))
            .is_err()
    );
    assert_eq!(
        mail.composer_window_event(&data(
            json!({"action":"draft_discard","id":"another-draft"})
        )),
        None
    );
    workspace.send("provider:mail", "mail-composer", "close".into());
    assert!(
        workspace
            .take_requests()
            .iter()
            .any(|(_, request)| matches!(
                request,
                moonframe::moonframe_core::Request::Window(
                    moonframe::moonframe_core::WindowRequest::Close(name)
                ) if name == "mail-composer"
            ))
    );
    workspace.send("shell", "shell", data(json!({"edit_identity":{"id":"form","account":"you@example.com","name":"You","address":"work@example.org","reply_to":"","signature":"Synthetic signature"}})));
    assert!(
        workspace.problems().is_empty(),
        "{:?}",
        workspace.problems()
    );
    mail.handle(data(json!({"action":"columns_edit","base":mail.snapshot()?["columns_key"],"operation":"toggle","field":"sender"})))?;
    mail.handle(data(json!({"action":"columns_edit","base":mail.snapshot()?["columns_key"],"operation":"toggle","field":"recipients"})))?;
    mail.handle(data(json!({"action":"columns_edit","base":mail.snapshot()?["columns_key"],"operation":"left","field":"date"})))?;
    workspace.set_host("mail", data(mail.snapshot()?));
    workspace.send("shell", "shell", "mail".into());
    workspace.send("settings", "settings", data(json!({"page":"reading"})));
    assert!(
        workspace.problems().is_empty(),
        "custom columns: {:?}",
        workspace.problems()
    );
    let inbox = mail
        .archive()?
        .role_mailbox("you@example.com", MailboxRole::Inbox)?;
    let fixture = mail.archive_mut()?.ingest(
        &moonmail_mime::StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        moonmail_store::IncomingMessage {
            account: "you@example.com",
            mailbox: inbox,
            origin: "attachment-layout",
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 1,
            raw: include_bytes!("../../moonmail-mime/tests/fixtures/attachments.eml"),
        },
    )?;
    mail.select(fixture, true)?;
    mail.handle(data(json!({"action":"attachments","message":fixture})))?;
    workspace.set_host("mail", data(mail.snapshot()?));
    workspace.send(
        "shell",
        "shell",
        data(json!({"action":"attachments","message":fixture})),
    );
    assert!(
        workspace.problems().is_empty(),
        "{:?}",
        workspace.problems()
    );
    workspace.take_outbox();
    workspace.take_requests();
    workspace.send("shell", "shell", data(json!({"attach_draft":draft_id})));
    let requests = workspace.take_requests();
    let [(_, moonframe::moonframe_core::Request::Pick(pick))] = requests.as_slice() else {
        bail!("Missing native attachment picker: {requests:?}");
    };
    assert!(pick.multiple);
    let answer = pick.answer(Some(vec![dir.path().join("synthetic.txt")]));
    workspace.send("shell", "shell", answer);
    let outgoing = workspace.take_outbox();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(
        outgoing[0].2.get("action").and_then(Data::as_str),
        Some("draft_attach")
    );
    assert_eq!(
        outgoing[0].2.get("id").and_then(Data::as_str),
        Some(draft_id.as_str())
    );
    workspace.send(
        "shell",
        "shell",
        data(json!({"save_attachment":"0.1","message":fixture,"name":"sample.bin"})),
    );
    let requests = workspace.take_requests();
    let [(_, moonframe::moonframe_core::Request::Pick(pick))] = requests.as_slice() else {
        bail!("Missing native export picker: {requests:?}");
    };
    assert_eq!(pick.name.as_deref(), Some("sample.bin"));
    workspace.send(
        "shell",
        "shell",
        pick.answer(Some(vec![dir.path().join("export.bin")])),
    );
    let outgoing = workspace.take_outbox();
    assert_eq!(outgoing.len(), 1);
    assert_eq!(
        outgoing[0].2.get("action").and_then(Data::as_str),
        Some("attachment_save")
    );
    assert_eq!(
        outgoing[0].2.get("message").and_then(Data::as_i64),
        Some(fixture.0 as i64)
    );
    workspace.send(
        "shell",
        "shell",
        data(json!({"action":"draft_attach","id":draft_id,"cancelled":true})),
    );
    assert!(
        workspace.take_outbox().is_empty(),
        "cancelled native pickers must not edit a draft"
    );
    assert!(
        mail.handle(data(json!({"action":"queue"}))).is_err(),
        "demo cannot send"
    );
    assert!(
        mail.handle(data(json!({"action":"sync"}))).is_err(),
        "demo cannot connect"
    );
    Ok(())
}

#[test]
fn column_edits_reject_stale_events_and_failed_saves_without_changing_navigation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    moonmail_store::seed_demo(&mut Archive::open(dir.path())?, &FileBlobs::new(dir.path()))?;
    let mut mail =
        Mail::new(dir.path().to_owned(), true).with_font_config(dir.path().join("ui/style.luau"));
    mail.initialize()?;
    let base = mail.snapshot()?["columns_key"].clone();
    let request =
        json!({"action":"columns_edit","base":base,"operation":"toggle","field":"sender"});
    let old_query = format!("{:?}", mail.query);
    let old_selected = mail.selected.as_ref().map(|detail| detail.summary.id);
    mail.handle(data(request.clone()))?;
    assert!(
        mail.handle(data(request)).is_err(),
        "old layout cannot toggle current state"
    );
    assert_eq!(format!("{:?}", mail.query), old_query);
    assert_eq!(
        mail.selected.as_ref().map(|detail| detail.summary.id),
        old_selected
    );
    let saved = mail.preferences.columns.clone();
    assert_eq!(crate::fonts::Preferences::load(dir.path())?.columns, saved);
    mail.handle(data(json!({"action":"settings_save","interface_family":".AppleSystemUIFont","list_family":"Menlo","font_size":"12","reader_family":"Helvetica","reader_size":"12"})))?;
    assert_eq!(
        mail.preferences.columns, saved,
        "other settings preserve column preferences"
    );
    std::fs::remove_file(dir.path().join("preferences.json"))?;
    std::fs::create_dir(dir.path().join("preferences.json"))?;
    assert!(mail.handle(data(json!({"action":"columns_edit","base":mail.snapshot()?["columns_key"],"operation":"reset"}))).is_err());
    assert_eq!(
        mail.preferences.columns, saved,
        "failed disk publication keeps current layout"
    );
    Ok(())
}

#[test]
fn attachment_actions_copy_files_atomically_bind_drafts_and_export_without_overwrite() -> Result<()>
{
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("you@example.com", "INBOX")?;
    let id = archive.ingest(
        &moonmail_mime::StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        moonmail_store::IncomingMessage {
            account: "you@example.com",
            mailbox: inbox,
            origin: "attachment-fixture",
            epoch: 0,
            uid: None,
            seen: true,
            starred: false,
            received_at: 1,
            raw: include_bytes!("../../moonmail-mime/tests/fixtures/attachments.eml"),
        },
    )?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    mail.reader_dir = Some(tempfile::tempdir_in(dir.path())?);
    mail.select(id, true)?;
    mail.handle(data(json!({"action":"attachments","message":id})))?;
    assert_eq!(mail.attachments.len(), 4);
    let part = mail.attachments[0].id.clone();
    assert_eq!(mail.attachments[0].name, "sample.bin");
    let destination = dir.path().join("saved.bin");
    let save = json!({"action":"attachment_save","message":id,"part":part,"path":destination});
    mail.handle(data(save.clone()))?;
    assert_eq!(std::fs::read(&destination)?, [0, 1, 2, 3, 255, 254]);
    std::fs::write(&destination, b"existing user file")?;
    assert!(mail.handle(data(save.clone())).is_err());
    assert_eq!(std::fs::read(&destination)?, b"existing user file");
    mail.select(id, true)?;
    assert!(
        mail.handle(data(save)).is_err(),
        "stale panels must not act after changing selection"
    );

    mail.handle(data(json!({"action":"compose"})))?;
    let draft_id = mail.draft.as_ref().context("missing draft")?.id.clone();
    let source = dir.path().join("notes.txt");
    std::fs::write(&source, b"original attachment")?;
    mail.handle(data(
        json!({"action":"draft_attach","id":draft_id,"paths":[source]}),
    ))?;
    let attached = mail.draft.as_ref().context("missing draft")?.attachments[0].clone();
    assert_eq!(
        FileBlobs::attachments(dir.path()).get(&attached.blob)?,
        b"original attachment"
    );
    std::fs::write(&source, b"changed source")?;
    assert_eq!(
        FileBlobs::attachments(dir.path()).get(&attached.blob)?,
        b"original attachment"
    );
    assert!(mail.handle(data(json!({"action":"draft_attach","id":draft_id,"paths":[source,dir.path().join("missing")]}))).is_err());
    assert_eq!(
        mail.archive()?
            .draft(&draft_id)?
            .context("missing saved draft")?
            .attachments
            .len(),
        1
    );
    assert_eq!(
        mail.draft
            .as_ref()
            .context("missing draft")?
            .attachments
            .len(),
        1
    );
    mail.handle(data(
        json!({"action":"draft_attach","id":draft_id,"cancelled":true}),
    ))?;
    mail.handle(data(json!({"action":"compose"})))?;
    assert!(
        mail.handle(data(
            json!({"action":"draft_attach","id":draft_id,"paths":[source]})
        ))
        .is_err()
    );
    assert!(
        mail.handle(data(
            json!({"action":"draft_detach","id":draft_id,"part":attached.id})
        ))
        .is_err()
    );
    std::fs::remove_file(&source)?;
    mail.handle(data(json!({"action":"draft_open","id":draft_id})))?;
    assert_eq!(
        mail.draft
            .as_ref()
            .context("missing reopened draft")?
            .attachments
            .len(),
        1
    );
    mail.handle(data(
        json!({"action":"draft_detach","id":draft_id,"part":attached.id}),
    ))?;
    assert!(
        mail.archive()?
            .draft(&draft_id)?
            .context("missing saved draft")?
            .attachments
            .is_empty()
    );
    Ok(())
}

#[test]
fn checked_messages_move_atomically_and_role_overrides_drive_archive_action() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    let mut archive = Archive::open(dir.path())?;
    moonmail_store::seed_demo(&mut archive, &FileBlobs::new(dir.path()))?;
    let inbox = archive.role_mailbox("you@example.com", MailboxRole::Inbox)?;
    let archive_folder = archive.role_mailbox("you@example.com", MailboxRole::Archive)?;
    let custom = archive.mailbox("you@example.com", "Filed manually")?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    mail.handle(data(json!({"action":"folder","id":inbox})))?;
    let ids: Vec<_> = mail.page.rows.iter().take(2).map(|row| row.id).collect();
    for id in &ids {
        mail.handle(data(json!({"action":"toggle_selection","id":id})))?;
    }
    assert_eq!(mail.snapshot()?["checked_count"], 2);
    mail.handle(data(
        json!({"action":"folder_role","id":custom,"role":"archive"}),
    ))?;
    mail.handle(data(json!({"action":"archive_messages"})))?;
    assert_eq!(mail.snapshot()?["checked_count"], 0);
    let moved = mail
        .archive()?
        .page(
            &MailQuery {
                mailbox: Some(custom),
                ..Default::default()
            },
            10,
        )?
        .rows;
    assert_eq!(moved.len(), 2);
    assert!(moved.iter().all(|row| ids.contains(&row.id)));
    assert!(mail.page.rows.iter().all(|row| !ids.contains(&row.id)));
    assert_eq!(
        mail.archive()?
            .role_mailbox("you@example.com", MailboxRole::Archive)?,
        custom
    );
    mail.handle(data(
        json!({"action":"folder_role","id":custom,"role":"default"}),
    ))?;
    assert_eq!(
        mail.archive()?
            .role_mailbox("you@example.com", MailboxRole::Archive)?,
        archive_folder
    );
    let visible = mail.page.rows[0].id;
    mail.handle(data(json!({"action":"toggle_selection","id":visible})))?;
    mail.handle(data(json!({"action":"folder","id":custom})))?;
    assert!(
        mail.checked.is_empty(),
        "new scope must not silently retain hidden selections"
    );
    assert!(
        mail.handle(data(json!({"action":"toggle_selection","id":visible})))
            .is_err()
    );
    Ok(())
}

#[test]
fn active_search_refreshes_after_indexing_and_applies_live_flag_filters() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    let archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("me@example.org", "INBOX")?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    mail.handle(data(json!({"action":"folder","id":inbox})))?;
    mail.handle(data(json!({"action":"search","value":"ccneedle"})))?;
    assert!(mail.page.rows.is_empty());
    let id = mail.archive_mut()?.ingest(
        &moonmail_mime::StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        moonmail_store::IncomingMessage {
            account: "me@example.org", mailbox: inbox, origin: "synthetic", epoch: 1,
            uid: Some(1), seen: false, starred: false, received_at: 1,
            raw: b"From: Nickname <sender@example.org>\r\nTo: me@example.org\r\nCc: ccneedle@example.org\r\nSubject: Synthetic\r\n\r\nBody",
        },
    )?;
    assert!(mail.index_pending()?);
    mail.refresh()?;
    assert_eq!(
        mail.page.rows[0].id, id,
        "active search includes newly indexed mail"
    );
    assert!(!mail.index_pending()?);
    mail.handle(data(json!({"action":"filter","value":"unread"})))?;
    assert_eq!(mail.page.rows.len(), 1);
    mail.archive_mut()?.set_flag(id, "seen", true)?;
    mail.refresh()?;
    assert!(
        mail.page.rows.is_empty(),
        "flags are not frozen in the search set"
    );
    mail.handle(data(json!({"action":"filter","value":"all"})))?;
    mail.handle(data(json!({"action":"search","value":"Nickname"})))?;
    assert_eq!(mail.page.rows[0].id, id);
    mail.handle(data(json!({"action":"search","value":"missing"})))?;
    assert!(mail.page.rows.is_empty());
    mail.handle(data(json!({"action":"search","value":""})))?;
    assert_eq!(mail.page.rows[0].id, id);
    Ok(())
}

#[test]
fn reply_action_propagates_references_and_falls_back_to_in_reply_to() -> Result<()> {
    use moonmail_core::MimeParser;
    let parser = moonmail_mime::StalwartParser::default();
    for (uid, ancestry) in [
        (
            1,
            "References: <root@example.org> <ancestor@example.org>\r\nIn-Reply-To: <ancestor@example.org>\r\n",
        ),
        (2, "In-Reply-To: <root@example.org>\r\n"),
    ] {
        let dir = tempfile::tempdir()?;
        let mut mail = Mail::new(dir.path().to_owned(), true);
        let archive = Archive::open(dir.path())?;
        let inbox = archive.mailbox("me@example.org", "INBOX")?;
        mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
        mail.archive = Some(archive);
        let raw = format!(
            "From: sender@example.org\r\nTo: me@example.org\r\nCc: peer@example.org\r\nMessage-ID: <parent{uid}@example.org>\r\nSubject: Synthetic\r\n{ancestry}\r\nBody"
        );
        let id = mail.archive_mut()?.ingest(
            &parser,
            &FileBlobs::new(dir.path()),
            moonmail_store::IncomingMessage {
                account: "me@example.org",
                mailbox: inbox,
                origin: &uid.to_string(),
                epoch: 1,
                uid: Some(uid),
                seen: false,
                starred: false,
                received_at: 1,
                raw: raw.as_bytes(),
            },
        )?;
        mail.selected = Some(mail.archive()?.detail(id)?);
        mail.handle(data(
            json!({"action":if uid == 1 {"reply"} else {"reply_all"}}),
        ))?;
        let draft_id = mail.draft.as_ref().context("No reply draft")?.id.clone();
        mail.handle(data(
            json!({"action":"draft","id":draft_id,"field":"body","value":"Synthetic reply"}),
        ))?;
        let draft = mail
            .archive()?
            .latest_draft()?
            .context("missing reply draft")?;
        let expected = if uid == 1 {
            vec!["root@example.org", "ancestor@example.org"]
        } else {
            vec!["root@example.org"]
        };
        assert_eq!(draft.references, expected);
        let account = Account::fastmail("me@example.org")?;
        let (raw, recipients) = moonmail_network::prepare_message(&account, &draft)?;
        assert_eq!(
            recipients,
            if uid == 1 {
                vec!["sender@example.org"]
            } else {
                vec!["sender@example.org", "peer@example.org"]
            }
        );
        let parsed = parser.parse(&raw)?;
        let mut expected: Vec<String> = expected.into_iter().map(str::to_owned).collect();
        expected.push(format!("parent{uid}@example.org"));
        assert_eq!(parsed.references, expected);
    }
    Ok(())
}

#[test]
fn independent_drafts_reject_stale_events_and_failed_edits_preserve_saved_content() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), false);
    let archive = Archive::open(dir.path())?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    Account::fastmail("me@example.org")?.save(dir.path())?;
    mail.handle(data(json!({"action":"compose"})))?;
    let first = mail
        .draft
        .as_ref()
        .context("missing first draft")?
        .id
        .clone();
    mail.handle(data(
        json!({"action":"draft","id":first,"field":"body","value":"First body"}),
    ))?;
    mail.handle(data(json!({"action":"compose"})))?;
    let second = mail
        .draft
        .as_ref()
        .context("missing second draft")?
        .id
        .clone();
    assert_ne!(first, second);
    mail.handle(data(
        json!({"action":"draft","id":first,"field":"body","value":"Stale body"}),
    ))?;
    assert!(
        mail.draft
            .as_ref()
            .context("missing second")?
            .body
            .is_empty()
    );
    assert_eq!(
        mail.archive()?.draft(&first)?.context("lost first")?.body,
        "First body"
    );
    assert!(
        mail.handle(data(json!({"action":"queue","id":first})))
            .is_err()
    );
    assert!(mail.archive()?.outbox()?.is_empty());
    mail.handle(data(json!({"action":"draft_open","id":first})))?;
    // A failed disk write must not make the provider advertise unsaved content.
    Account::fastmail("other@example.org")?.save(dir.path())?;
    let connection = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    connection.execute_batch("CREATE TRIGGER reject_edit BEFORE UPDATE ON drafts BEGIN SELECT RAISE(ABORT,'synthetic write failure'); END;")?;
    assert!(
        mail.handle(data(
            json!({"action":"draft","id":first,"field":"body","value":"Unsaved edit"})
        ))
        .is_err()
    );
    assert_eq!(
        mail.draft.as_ref().context("lost active")?.body,
        "First body"
    );
    assert!(
        mail.handle(data(
            json!({"action":"draft_account","id":first,"account":"other@example.org"})
        ))
        .is_err()
    );
    assert_eq!(
        mail.draft.as_ref().context("lost active")?.account,
        "me@example.org"
    );
    assert_eq!(
        mail.archive()?
            .draft(&first)?
            .context("lost saved")?
            .account,
        "me@example.org"
    );
    connection.execute_batch("DROP TRIGGER reject_edit;")?;
    mail.handle(data(
        json!({"action":"draft","id":first,"field":"recipients","value":"peer@example.org"}),
    ))?;
    Account::remove(dir.path(), "me@example.org")?;
    assert!(
        mail.handle(data(json!({"action":"queue","id":first})))
            .is_err()
    );
    assert!(mail.archive()?.outbox()?.is_empty());
    assert!(mail.job.is_none());
    mail.handle(data(json!({"action":"draft_discard","id":first})))?;
    mail.handle(data(
        json!({"action":"draft","id":first,"field":"body","value":"Resurrected"}),
    ))?;
    assert!(mail.draft.is_none());
    assert!(mail.archive()?.draft(&first)?.is_none());
    assert!(
        mail.handle(data(json!({"action":"draft_open","id":first})))
            .is_err()
    );
    assert!(mail.archive()?.draft(&second)?.is_some());
    for _ in 0..51 {
        mail.handle(data(json!({"action":"compose"})))?;
    }
    let snapshot = mail.snapshot()?;
    assert_eq!(snapshot["draft_count"], 52);
    assert_eq!(
        snapshot["drafts"]["rows"]
            .as_array()
            .context("missing rows")?
            .len(),
        50
    );
    mail.handle(data(json!({"action":"drafts_next"})))?;
    assert_eq!(
        mail.snapshot()?["drafts"]["rows"]
            .as_array()
            .context("missing next page")?
            .len(),
        2
    );
    mail.handle(data(json!({"action":"drafts_first"})))?;
    assert!(mail.draft_before.is_none());
    Ok(())
}

#[test]
fn queue_only_restoration_and_cancellation_preserve_other_drafts_without_network() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let mut mail = Mail::new(dir.path().to_owned(), true);
    let archive = Archive::open(dir.path())?;
    mail.index = Some(crate::local::open_index(dir.path(), &archive)?);
    mail.archive = Some(archive);
    mail.handle(data(json!({"action":"compose"})))?;
    let draft_id = mail.draft.as_ref().context("missing draft")?.id.clone();
    for (field, value) in [
        ("recipients", "Peer <peer@example.org>"),
        ("bcc", "hidden@example.org"),
        ("body", "Saved body"),
    ] {
        mail.handle(data(
            json!({"action":"draft","id":draft_id,"field":field,"value":value}),
        ))?;
    }
    let path = dir.path().join("file.txt");
    std::fs::write(&path, b"Saved attachment")?;
    mail.handle(data(
        json!({"action":"draft_attach","id":draft_id,"paths":[path]}),
    ))?;
    let expected = mail.draft.clone().context("missing draft")?;
    mail.handle(data(json!({"action":"queue_only","id":draft_id})))?;
    assert!(mail.draft.is_none() && mail.job.is_none());
    let item = mail.archive()?.outbox()?.remove(0);
    assert_eq!(item.state, "queued");
    assert_eq!(item.recipients, ["peer@example.org", "hidden@example.org"]);
    mail.handle(data(json!({"action":"compose"})))?;
    let other = mail
        .draft
        .as_ref()
        .context("missing other draft")?
        .id
        .clone();
    mail.handle(data(json!({"action":"outbox_edit","id":item.id})))?;
    let restored = mail.draft.as_ref().context("missing restored draft")?;
    assert_ne!(restored.id, expected.id);
    assert_eq!(restored.body, expected.body);
    assert_eq!(restored.bcc, expected.bcc);
    assert_eq!(restored.attachments[0].blob, expected.attachments[0].blob);
    let restored_id = restored.id.clone();
    assert!(mail.archive()?.draft(&other)?.is_some());
    assert!(
        mail.handle(data(json!({"action":"outbox_edit","id":item.id})))
            .is_err()
    );
    mail.handle(data(json!({"action":"queue_only","id":restored_id})))?;
    let second = mail.archive()?.outbox()?.remove(0);
    mail.handle(data(json!({"action":"outbox_cancel","id":second.id})))?;
    assert_eq!(mail.archive()?.outbox_item(second.id)?.state, "cancelled");
    assert!(
        mail.handle(data(json!({"action":"outbox_retry","id":second.id})))
            .is_err()
    );
    assert!(mail.job.is_none());
    Ok(())
}
