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
        #[cfg(feature = "capture")]
        "capture-snapshot" => {
            let view = args.get(1).context("Missing capture view")?;
            println!(
                "{}",
                serde_json::to_string(&crate::provider::Mail::capture_snapshot(root, view)?)?
            );
        },
        "login" | "password" => {
            let email = args
                .get(1)
                .context("Usage: moonmail login EMAIL (or password EMAIL)")?;
            let provider = args
                .get(2)
                .map(String::as_str)
                .or_else(|| {
                    email
                        .rsplit_once('@')
                        .and_then(|(_, domain)| crate::account_setup::domain_provider(domain))
                })
                .unwrap_or("fastmail");
            let account = Account::preset(crate::account_setup::provider(provider)?, email)?;
            account.check_registration(root)?;
            let auth = Auth::new(Keychain);
            if command == "login" {
                auth.authorize_provider(account.provider, email)?;
                moonmail_network::check_account(&account, &auth.credentials_for(&account)?)?;
            } else {
                let secret = moonmail_network::LoginSecret {
                    bearer: false,
                    value: rpassword::prompt_password("App password: ")?,
                };
                moonmail_network::check_account(&account, &secret)?;
                auth.password(email, secret.value.clone())?;
            }
            account.save(root)?;
            println!("Connected {email}. Run `just sync` to download mail.");
        },
        "connect" => {
            let path = args
                .get(1)
                .context("Usage: moonmail connect ACCOUNT.json")?;
            let account: Account = serde_json::from_slice(&std::fs::read(path)?)?;
            account.check_registration(root)?;
            let secret = moonmail_network::LoginSecret {
                bearer: false,
                value: rpassword::prompt_password("Mail server password: ")?,
            };
            moonmail_network::check_account(&account, &secret)?;
            Auth::new(Keychain).password(&account.email, secret.value.clone())?;
            account.save(root)?;
            println!(
                "Connected {}. Run `just sync` to download mail.",
                account.email
            );
        },
        "oauth-client" => {
            use std::io::Read;
            let provider = crate::account_setup::provider(
                args.get(1)
                    .context("Usage: moonmail oauth-client PROVIDER CLIENT.json")?,
            )?;
            let path = args.get(2).context("Choose an OAuth client JSON file")?;
            let mut bytes = Vec::new();
            std::fs::File::open(path)?
                .take(65537)
                .read_to_end(&mut bytes)?;
            Auth::new(Keychain).import_oauth_client(provider, &bytes)?;
            println!("OAuth desktop client saved in macOS Keychain.");
        },
        "disconnect" => {
            let account = if let Some(email) = args.get(1) {
                Account::load_for(root, email)?
                    .context("Account is not connected")?
                    .into()
            } else {
                Account::load(root)?
            };
            if let Some(account) = account {
                Auth::new(Keychain).disconnect(&account.email)?;
                Account::remove(root, &account.email)?;
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
        "export" => {
            anyhow::ensure!(
                args.len() == 3,
                "Usage: moonmail export MESSAGE_ID DESTINATION.eml"
            );
            let id = args[1]
                .parse::<u64>()
                .context("Message ID must be a positive local ID")?;
            anyhow::ensure!(id > 0 && id <= i64::MAX as u64, "Invalid local message ID");
            let destination = std::path::absolute(&args[2])?;
            crate::file_export::message(
                &Archive::open(root)?,
                &FileBlobs::new(root),
                moonmail_core::MessageId(id),
                &destination,
            )?;
            println!("Saved original message to {}", destination.display());
        },
        "export-many" => {
            anyhow::ensure!(
                (3..=102).contains(&args.len()),
                "Usage: moonmail export-many DIRECTORY MESSAGE_ID [MESSAGE_ID ...] (up to 100)"
            );
            let ids = args[2..]
                .iter()
                .map(|value| {
                    let id = value
                        .parse::<u64>()
                        .context("Message ID must be a positive local ID")?;
                    anyhow::ensure!(id > 0 && id <= i64::MAX as u64, "Invalid local message ID");
                    Ok(moonmail_core::MessageId(id))
                })
                .collect::<Result<Vec<_>>>()?;
            let destination = crate::file_export::messages(
                &Archive::open(root)?,
                &FileBlobs::new(root),
                &ids,
                &std::path::absolute(&args[1])?,
            )?;
            println!("Exported originals to {}", destination.display());
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
            "Moonmail\n\n  (no arguments)                Open the mail client\n  login EMAIL [PROVIDER]         Browser OAuth + PKCE; provider inferred from domain\n  password EMAIL [PROVIDER]      Verify IMAP/SMTP and store an app password\n  connect ACCOUNT.json           Connect custom IMAP/SMTP using a hidden password prompt\n  oauth-client PROVIDER FILE     Import Google/Microsoft desktop client JSON into Keychain\n  sync                          Sync all accounts and finish pending Sent copies\n  import PATH                   Import .eml files recursively\n  export ID PATH                Save original bytes as a new .eml file\n  export-many DIR IDs            Export up to 100 originals into a new folder\n  reindex                       Replay the search journal\n  status                        Show local archive counts\n  disconnect [EMAIL]            Disconnect one account; retain local mail\n  --check                       Isolated Moonframe UI validation\n\nProviders: fastmail, google, microsoft, icloud, yahoo, aol, other.\nGoogle/Microsoft browser login requires an imported desktop OAuth client.\nMOONMAIL_DATA_DIR overrides the archive directory. MOONMAIL_DEMO=1 selects synthetic demo mail."
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
