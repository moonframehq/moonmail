use moonmail_network::Account;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn provider_presets_and_legacy_accounts_keep_explicit_transport_semantics() -> Result {
    use moonmail_network::{Provider, SentCopy, TlsMode};
    let dir = tempfile::tempdir()?;
    std::fs::write(dir.path().join("account.json"), br#"{"email":"old@example.org","imap_host":"imap.fastmail.com","imap_port":993,"smtp_host":"smtp.fastmail.com","smtp_port":465}"#)?;
    let legacy = Account::load(dir.path())?.ok_or("missing legacy account")?;
    assert_eq!(legacy.provider, Provider::Fastmail);
    assert_eq!(legacy.imap_tls, TlsMode::Implicit);
    assert_eq!(legacy.smtp_login(), legacy.email);
    assert_eq!(legacy.sent_copy, SentCopy::Server);
    for provider in [
        Provider::Fastmail,
        Provider::Google,
        Provider::Microsoft,
        Provider::Icloud,
        Provider::Yahoo,
        Provider::Aol,
    ] {
        let account = Account::preset(provider, "custom@example.org")?;
        account.validate()?;
        account.save(dir.path())?;
        assert_eq!(
            Account::load_for(dir.path(), &account.email)?
                .ok_or("missing")?
                .provider,
            provider
        );
    }
    let gmail = Account::preset(Provider::Google, "me@gmail.com")?;
    assert_eq!(gmail.smtp_host, "smtp.gmail.com");
    assert_eq!(gmail.sent_copy, SentCopy::Server);
    assert_eq!(
        Account::preset(Provider::Microsoft, "me@Outlook.com")?.smtp_host,
        "smtp-mail.outlook.com"
    );
    assert_eq!(
        Account::preset(Provider::Microsoft, "me@company.example")?.smtp_host,
        "smtp.office365.com"
    );
    let icloud = Account::preset(Provider::Icloud, "me@icloud.com")?;
    assert_eq!(
        (icloud.smtp_port, icloud.smtp_tls),
        (587, TlsMode::Starttls)
    );
    assert_eq!(icloud.sent_copy, SentCopy::Append);
    let mut custom = Account::preset(Provider::Other, "me@example.org")?;
    assert!(custom.validate().is_err());
    custom.imap_host = "mail.example.org".into();
    custom.smtp_host = "relay.example.org".into();
    custom.imap_username = "mailbox-login".into();
    custom.smtp_username = "submission-login".into();
    custom.validate()?;
    assert_eq!(custom.imap_login(), "mailbox-login");
    assert_eq!(custom.smtp_login(), "submission-login");
    custom.imap_host = "https://mail.example.org:993".into();
    assert!(custom.validate().is_err());
    assert!(serde_json::from_value::<Account>(serde_json::json!({"email":"me@example.org","imap_host":"mail.example.org","imap_port":143,"imap_tls":"plaintext","smtp_host":"mail.example.org","smtp_port":25})).is_err());
    Ok(())
}

#[test]
fn legacy_registry_migrates_on_write_and_keeps_registration_order() -> Result {
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    assert!(Account::load(root)?.is_none());
    let mut first = Account::fastmail("first@example.org")?;
    let second = Account::fastmail("second@example.org")?;
    let legacy = serde_json::to_vec(&first)?;
    std::fs::write(root.join("account.json"), &legacy)?;
    assert_eq!(
        Account::load(root)?.ok_or("missing legacy")?.email,
        first.email
    );
    assert_eq!(std::fs::read(root.join("account.json"))?, legacy);
    second.save(root)?;
    first.smtp_host = "smtp.example.org".into();
    first.save(root)?;
    let accounts = Account::load_all(root)?;
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0].email, first.email);
    assert_eq!(accounts[0].smtp_host, first.smtp_host);
    assert_eq!(accounts[1].email, second.email);
    assert!(Account::load_for(root, "missing@example.org")?.is_none());
    assert!(Account::load_for(root, "")?.is_none());
    assert_eq!(
        Account::load_for(root, &second.email)?
            .ok_or("missing second")?
            .email,
        second.email
    );
    Account::remove(root, &first.email)?;
    assert_eq!(
        Account::load(root)?.ok_or("missing default")?.email,
        second.email
    );
    assert!(Account::load_for(root, &first.email)?.is_none());
    Account::remove(root, &second.email)?;
    assert!(Account::load_all(root)?.is_empty());
    Ok(())
}

#[test]
fn rejected_registry_updates_preserve_the_previous_file() -> Result {
    let dir = tempfile::tempdir()?;
    for index in 0..32 {
        Account::fastmail(&format!("account{index}@example.org"))?.save(dir.path())?;
    }
    let path = dir.path().join("account.json");
    let before = std::fs::read(&path)?;
    assert!(
        Account::fastmail("overflow@example.org")?
            .save(dir.path())
            .is_err()
    );
    assert_eq!(std::fs::read(&path)?, before);
    let mut invalid = Account::fastmail("account0@example.org")?;
    invalid.email = "invalid\r\naddress".into();
    assert!(invalid.save(dir.path()).is_err());
    assert_eq!(std::fs::read(&path)?, before);
    // Updating an existing account still works at capacity.
    Account::fastmail("account0@example.org")?.save(dir.path())?;
    assert_eq!(Account::load_all(dir.path())?.len(), 32);
    Ok(())
}

#[test]
fn malformed_or_duplicate_registry_is_never_silently_overwritten() -> Result {
    let dir = tempfile::tempdir()?;
    let account = Account::fastmail("you@example.org")?;
    for bytes in [
        b"{broken".to_vec(),
        serde_json::to_vec(&serde_json::json!({"accounts":[account, account]}))?,
        serde_json::to_vec(&serde_json::json!({"accounts":vec![&account; 33]}))?,
    ] {
        let path = dir.path().join("account.json");
        std::fs::write(&path, &bytes)?;
        assert!(Account::load_all(dir.path()).is_err());
        assert!(account.save(dir.path()).is_err());
        assert!(Account::remove(dir.path(), &account.email).is_err());
        assert_eq!(std::fs::read(&path)?, bytes);
    }
    Ok(())
}
