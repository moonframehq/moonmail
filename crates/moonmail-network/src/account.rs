use {
    crate::{Error, Result},
    serde::{Deserialize, Serialize},
    std::{collections::HashSet, fs, io::Write, path::Path},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    #[serde(default)]
    pub provider: Provider,
    pub imap_host: String,
    pub imap_port: u16,
    #[serde(default)]
    pub imap_tls: TlsMode,
    #[serde(default)]
    pub imap_username: String,
    pub smtp_host: String,
    pub smtp_port: u16,
    #[serde(default)]
    pub smtp_tls: TlsMode,
    #[serde(default)]
    pub smtp_username: String,
    #[serde(default)]
    pub sent_copy: SentCopy,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SentCopy {
    #[default]
    Server,
    Append,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    #[default]
    Fastmail,
    Google,
    Microsoft,
    Icloud,
    Yahoo,
    Aol,
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TlsMode {
    #[default]
    Implicit,
    Starttls,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum SavedAccounts {
    Collection { accounts: Vec<Account> },
    Legacy(Account),
}

const MAX_ACCOUNTS: usize = 32;

impl Account {
    pub fn fastmail(email: &str) -> Result<Self> {
        Self::preset(Provider::Fastmail, email)
    }

    pub fn preset(provider: Provider, email: &str) -> Result<Self> {
        validate_address(email)?;
        let (imap, smtp, smtp_port, smtp_tls) = match provider {
            Provider::Fastmail => (
                "imap.fastmail.com",
                "smtp.fastmail.com",
                465,
                TlsMode::Implicit,
            ),
            Provider::Google => ("imap.gmail.com", "smtp.gmail.com", 465, TlsMode::Implicit),
            Provider::Microsoft => (
                "outlook.office365.com",
                if email.rsplit_once('@').is_some_and(|(_, domain)| {
                    matches!(
                        domain.to_ascii_lowercase().as_str(),
                        "outlook.com" | "hotmail.com" | "live.com" | "msn.com"
                    )
                }) {
                    "smtp-mail.outlook.com"
                } else {
                    "smtp.office365.com"
                },
                587,
                TlsMode::Starttls,
            ),
            Provider::Icloud => (
                "imap.mail.me.com",
                "smtp.mail.me.com",
                587,
                TlsMode::Starttls,
            ),
            Provider::Yahoo => (
                "imap.mail.yahoo.com",
                "smtp.mail.yahoo.com",
                465,
                TlsMode::Implicit,
            ),
            Provider::Aol => ("imap.aol.com", "smtp.aol.com", 465, TlsMode::Implicit),
            Provider::Other => ("", "", 587, TlsMode::Starttls),
        };
        Ok(Self {
            email: email.to_owned(),
            provider,
            imap_host: imap.into(),
            imap_port: 993,
            imap_tls: TlsMode::Implicit,
            imap_username: String::new(),
            smtp_host: smtp.into(),
            smtp_port,
            smtp_tls,
            smtp_username: String::new(),
            sent_copy: if matches!(provider, Provider::Google | Provider::Fastmail) {
                SentCopy::Server
            } else {
                SentCopy::Append
            },
        })
    }

    pub fn imap_login(&self) -> &str {
        if self.imap_username.is_empty() {
            &self.email
        } else {
            &self.imap_username
        }
    }

    pub fn smtp_login(&self) -> &str {
        if self.smtp_username.is_empty() {
            &self.email
        } else {
            &self.smtp_username
        }
    }

    pub fn validate(&self) -> Result<()> {
        validate_address(&self.email)?;
        for host in [&self.imap_host, &self.smtp_host] {
            if host.is_empty()
                || host.len() > 253
                || !host.split('.').all(|part| {
                    !part.is_empty()
                        && part.len() <= 63
                        && !part.starts_with('-')
                        && !part.ends_with('-')
                        && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                })
            {
                return Err(Error::Invalid(
                    "Enter a server hostname, without a URL or port",
                ));
            }
        }
        if self.imap_port == 0 || self.smtp_port == 0 {
            return Err(Error::Invalid("Server ports must be between 1 and 65535"));
        }
        for username in [&self.imap_username, &self.smtp_username] {
            if username.len() > 320 || username.chars().any(char::is_control) {
                return Err(Error::Invalid("Invalid mail server username"));
            }
        }
        Ok(())
    }

    /// Preflight registry validation before any browser or credential operation.
    pub fn check_registration(&self, root: &Path) -> Result<()> {
        let mut accounts = Self::load_all(root)?;
        accounts.retain(|account| account.email != self.email);
        accounts.push(self.clone());
        Self::validate_accounts(&accounts)
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        Ok(Self::load_all(root)?.into_iter().next())
    }

    /// Registration order is stable; the first account is the default. Legacy
    /// single-account files are read without modification until the next write.
    pub fn load_all(root: &Path) -> Result<Vec<Self>> {
        match fs::read(root.join("account.json")) {
            Ok(bytes) => {
                let accounts = match serde_json::from_slice(&bytes)? {
                    SavedAccounts::Collection { accounts } => accounts,
                    SavedAccounts::Legacy(account) => vec![account],
                };
                Self::validate_accounts(&accounts)?;
                Ok(accounts)
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    /// Resolve the owning account exactly. Never substitute the default for a
    /// disconnected draft or outbox account.
    pub fn load_for(root: &Path, email: &str) -> Result<Option<Self>> {
        Ok(Self::load_all(root)?
            .into_iter()
            .find(|account| account.email == email))
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        let mut accounts = Self::load_all(root)?;
        if let Some(account) = accounts
            .iter_mut()
            .find(|account| account.email == self.email)
        {
            *account = self.clone();
        } else {
            accounts.push(self.clone());
        }
        Self::persist(root, &accounts)
    }

    /// Removing an account retains all local mail and frozen drafts/outbox.
    pub fn remove(root: &Path, email: &str) -> Result<()> {
        let mut accounts = Self::load_all(root)?;
        accounts.retain(|account| account.email != email);
        Self::persist(root, &accounts)
    }

    fn validate_accounts(accounts: &[Self]) -> Result<()> {
        if accounts.len() > MAX_ACCOUNTS {
            return Err(Error::Invalid(
                "At most 32 connected accounts are supported",
            ));
        }
        let mut addresses = HashSet::new();
        for account in accounts {
            account.validate()?;
            if !addresses.insert(&account.email) {
                return Err(Error::Invalid("Duplicate connected account"));
            }
        }
        Ok(())
    }

    // Writers run under the app's archive lock. Publish the entire registry in
    // one atomic replacement, preserving the previous file on any failure.
    fn persist(root: &Path, accounts: &[Self]) -> Result<()> {
        Self::validate_accounts(accounts)?;
        fs::create_dir_all(root)?;
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(&serde_json::to_vec_pretty(
            &serde_json::json!({"accounts": accounts}),
        )?)?;
        file.as_file().sync_all()?;
        file.persist(root.join("account.json"))
            .map_err(|error| error.error)?;
        Ok(())
    }
}

pub fn validate_address(address: &str) -> Result<()> {
    if address.contains(['\r', '\n', '\0']) || !address.is_ascii() {
        return Err(Error::Invalid(
            "Use an ASCII email address without a display name",
        ));
    }
    let addresses = mailparse::addrparse(address)?;
    if addresses.len() != 1 {
        return Err(Error::Invalid("Enter one email address"));
    }
    match &addresses[0] {
        mailparse::MailAddr::Single(single)
            if single.addr == address
                && address
                    .rsplit_once('@')
                    .is_some_and(|(local, domain)| !local.is_empty() && !domain.is_empty()) =>
        {
            Ok(())
        },
        _ => Err(Error::Invalid(
            "Enter a bare email address, such as you@example.com",
        )),
    }
}
