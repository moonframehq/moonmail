use {
    anyhow::{Context, Result, bail},
    moonmail_core::{BlobStore, MailQuery, MailRepository},
    moonmail_network::{Account, Auth, Keychain},
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
    std::path::Path,
};

pub fn dispatch(root: &Path, args: &[String]) -> Result<bool> {
    let Some(command) = args.first().map(String::as_str) else {
        return Ok(false);
    };
    match command {
        "login" | "password" => {
            let email = args
                .get(1)
                .context("Usage: moonmail login EMAIL (or password EMAIL)")?;
            let account = Account::fastmail(email)?;
            let auth = Auth::new(Keychain, root);
            if command == "login" {
                auth.authorize(email)?;
            } else {
                auth.password(
                    email,
                    rpassword::prompt_password("Fastmail app password: ")?,
                )?;
            }
            account.save(root)?;
            println!("Connected {email}. Run `just sync` to download mail.");
        },
        "disconnect" => {
            if let Some(account) = Account::load(root)? {
                Auth::new(Keychain, root).disconnect(&account.email)?;
                std::fs::remove_file(root.join("account.json"))?;
            }
            println!("Disconnected. Local mail is retained.");
        },
        "sync" => {
            moonmail_network::sync_account(root, |status| println!("{status}"))?;
            crate::local::index_all(root)?;
        },
        "reindex" => {
            Archive::open(root)?.reset_index()?;
            crate::local::index_all(root)?;
            println!("Search projection rebuilt.");
        },
        "import" => {
            let path = Path::new(
                args.get(1)
                    .context("Usage: moonmail import PATH (an .eml file or directory)")?,
            );
            let mut archive = Archive::open(root)?;
            let blobs = FileBlobs::new(root);
            let mailbox = archive.mailbox("local", "Imported")?;
            let mut count = 0;
            import(path, &mut archive, &blobs, mailbox, &mut count)?;
            crate::local::index_all(root)?;
            println!("Imported {count} messages.");
        },
        "seed-demo" => {
            moonmail_store::seed_demo(&mut Archive::open(root)?, &FileBlobs::new(root))?;
            crate::local::index_all(root)?;
        },
        "status" => {
            let archive = Archive::open(root)?;
            println!(
                "Archive: {}\nMessages: {}",
                root.display(),
                archive.total()?
            );
            for mailbox in archive.mailboxes()? {
                println!(
                    "{}: {} messages, {} unread",
                    mailbox.name, mailbox.total, mailbox.unread
                );
            }
        },
        "smoke" => {
            let mut archive = Archive::open(root)?;
            moonmail_store::seed_demo(&mut archive, &FileBlobs::new(root))?;
            crate::local::index_all(root)?;
            let page = archive.page(&MailQuery::default(), 20)?;
            anyhow::ensure!(
                page.rows.len() == 20 && page.next.is_some(),
                "demo pagination failed"
            );
            let detail = archive.detail(page.rows[0].id)?;
            anyhow::ensure!(
                !detail.text.is_empty()
                    && archive.correspondence(detail.summary.id, 100)?.len() > 1,
                "reader/correspondence failed"
            );
            println!("Demo archive, search journal, pagination, reader and Correspondence passed.");
        },
        "--help" | "help" => println!(
            "Moonmail\n\n  (no arguments)       Open the mail client\n  login EMAIL          Fastmail OAuth (browser + PKCE)\n  password EMAIL       Store a Fastmail app password in Keychain\n  sync                 Download folders and reconcile flags\n  import PATH          Import .eml files recursively\n  reindex              Replay the search journal\n  status               Show local archive counts\n  disconnect           Revoke credentials; retain local mail\n  --check              Isolated Moonframe UI validation\n\nMOONMAIL_DATA_DIR overrides the archive directory. MOONMAIL_DEMO=1 selects synthetic demo mail."
        ),
        value if value.starts_with('-') || value == "bundle" => return Ok(false),
        _ => bail!("Unknown command {command}; use --help"),
    }
    Ok(true)
}

fn import(
    path: &Path,
    archive: &mut Archive,
    blobs: &FileBlobs,
    mailbox: i64,
    count: &mut usize,
) -> Result<()> {
    if path.is_symlink() {
        return Ok(());
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            import(&entry?.path(), archive, blobs, mailbox, count)?;
        }
    } else if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("eml"))
    {
        anyhow::ensure!(
            path.metadata()?.len() <= 64 * 1024 * 1024,
            "Message exceeds 64 MiB: {}",
            path.display()
        );
        let raw = std::fs::read(path)?;
        let hash = blobs.put(&raw)?;
        archive.ingest(
            &moonmail_mime::StalwartParser::default(),
            blobs,
            IncomingMessage {
                account: "local",
                mailbox,
                origin: &format!("import:{hash}"),
                epoch: 0,
                uid: None,
                seen: true,
                starred: false,
                received_at: 0,
                raw: &raw,
            },
        )?;
        *count += 1;
    }
    Ok(())
}
