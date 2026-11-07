use super::*;

impl Mail {
    pub(super) fn handle_account(&mut self, action: &str, message: &Data) -> Result<()> {
        tracing::debug!(
            action,
            provider = message.get("provider").and_then(Data::as_str),
            session = %crate::account_setup::session(),
            "account action"
        );
        let text = |key| {
            message
                .get(key)
                .and_then(Data::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        if action == "oauth_client_import" {
            let provider = crate::account_setup::provider(&text("provider"))?;
            if message.get("cancelled").and_then(Data::as_bool) == Some(true) {
                return Ok(());
            }
            let path = message
                .get("paths")
                .and_then(|data| {
                    if let Data::List(paths) = data {
                        paths.first()
                    } else {
                        None
                    }
                })
                .and_then(Data::as_str)
                .or_else(|| message.get("path").and_then(Data::as_str))
                .context("Choose an OAuth client JSON file")?
                .to_owned();
            return self.launch(move |_, _| {
                use std::io::Read;
                let mut bytes = Vec::new();
                std::fs::File::open(path)?
                    .take(65537)
                    .read_to_end(&mut bytes)?;
                Auth::new(Keychain).import_oauth_client(provider, &bytes)?;
                Ok(())
            });
        }
        if action == "account_check" {
            let account = Account::load_for(&self.root, &text("account"))?
                .context("Choose a connected account")?;
            return self.launch(move |_, progress| {
                let _ = progress.try_send(Event::Progress(
                    "Checking IMAP and SMTP authentication…".into(),
                ));
                let secret = Auth::new(Keychain).credentials_for(&account)?;
                moonmail_network::check_account(&account, &secret)?;
                Ok(())
            });
        }
        let provider = crate::account_setup::provider(&text("provider"))?;
        let email = text("email");
        let mut account = Account::preset(provider, &email)?;
        if let Some(config) = message.get("config") {
            let value = |key| config.get(key).and_then(Data::as_str).unwrap_or_default();
            let tls = |key| -> Result<moonmail_network::TlsMode> {
                match value(key) {
                    "implicit" => Ok(moonmail_network::TlsMode::Implicit),
                    "starttls" => Ok(moonmail_network::TlsMode::Starttls),
                    _ => bail!("Choose TLS or STARTTLS"),
                }
            };
            account.imap_host = value("imap_host").to_owned();
            account.smtp_host = value("smtp_host").to_owned();
            account.imap_port = value("imap_port").parse().context("Invalid IMAP port")?;
            account.smtp_port = value("smtp_port").parse().context("Invalid SMTP port")?;
            account.imap_tls = tls("imap_tls")?;
            account.smtp_tls = tls("smtp_tls")?;
            account.imap_username = value("imap_username").to_owned();
            account.smtp_username = value("smtp_username").to_owned();
            account.sent_copy = match value("sent_copy") {
                "append" => moonmail_network::SentCopy::Append,
                "server" => moonmail_network::SentCopy::Server,
                "" => account.sent_copy,
                _ => bail!("Choose how Sent copies are saved"),
            };
        }
        // OAuth grants must never be sent to manually substituted hosts.
        if action == "login" {
            let preset = Account::preset(provider, &email)?;
            anyhow::ensure!(
                account.imap_host == preset.imap_host
                    && account.smtp_host == preset.smtp_host
                    && account.imap_port == preset.imap_port
                    && account.smtp_port == preset.smtp_port
                    && account.imap_tls == preset.imap_tls
                    && account.smtp_tls == preset.smtp_tls
                    && account.imap_login() == email
                    && account.smtp_login() == email,
                "Use provider defaults for browser sign-in; custom servers use password sign-in"
            );
        }
        account.check_registration(&self.root)?;
        let password_login = action == "account_password";
        self.launch(move |root, progress| {
            let auth = Auth::new(Keychain);
            let secret = if password_login {
                moonmail_network::LoginSecret {
                    bearer: false,
                    value: crate::account_setup::prompt_password(&email)?.into(),
                }
            } else {
                auth.authorize_provider(provider, &email)?;
                auth.credentials_for(&account)?
            };
            let _ = progress.try_send(Event::Progress(
                "Checking IMAP and SMTP authentication…".into(),
            ));
            moonmail_network::check_account(&account, &secret)?;
            if password_login {
                auth.password(&email, secret.value.clone())?;
            }
            account.save(&root)?;
            Ok(())
        })
    }
}
