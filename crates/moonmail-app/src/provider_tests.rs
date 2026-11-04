use {
    super::*,
    moonframe::moonframe_core::{AppLayer, Roots, Workspace},
    serde_json::Value,
};

fn data(value: Value) -> Data {
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
    for page in ["appearance", "fonts", "reading", "account", "storage"] {
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
    assert!(
        workspace.take_outbox().is_empty(),
        "unsupported providers must never initiate Fastmail OAuth"
    );
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
    mail.handle(data(
        json!({"action":"draft","field":"body","value":"Synthetic reply"}),
    ))?;
    assert_eq!(
        mail.archive()?
            .latest_draft()?
            .context("No saved draft")?
            .body,
        "Synthetic reply"
    );
    workspace.set_host("mail", data(mail.snapshot()?));
    for pane in ["compose", "settings", "outbox", "mail", "rich", "thread"] {
        workspace.send("shell", "shell", pane.into());
        assert!(
            workspace.problems().is_empty(),
            "{pane}: {:?}",
            workspace.problems()
        );
    }
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
