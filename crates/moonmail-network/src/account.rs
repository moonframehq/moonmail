use {
    crate::{Error, Result},
    serde::{Deserialize, Serialize},
    std::{fs, io::Write, path::Path},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Account {
    pub email: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
}

impl Account {
    pub fn fastmail(email: &str) -> Result<Self> {
        validate_address(email)?;
        Ok(Self {
            email: email.to_owned(),
            imap_host: "imap.fastmail.com".into(),
            imap_port: 993,
            smtp_host: "smtp.fastmail.com".into(),
            smtp_port: 465,
        })
    }

    pub fn load(root: &Path) -> Result<Option<Self>> {
        match fs::read(root.join("account.json")) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    pub fn save(&self, root: &Path) -> Result<()> {
        fs::create_dir_all(root)?;
        let mut file = tempfile::NamedTempFile::new_in(root)?;
        file.write_all(&serde_json::to_vec_pretty(self)?)?;
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
