use std::{error::Error, fs::File};
#[cfg(any(target_os = "macos", test))]
use {
    fs2::FileExt,
    sha2::{Digest, Sha256},
    std::{fs::OpenOptions, path::Path},
};

#[cfg(any(target_os = "macos", test))]
const SERVICE: &str = "dev.moonmail.account";

/// Credentials and their exclusion guard share the same identity scope.
/// `Auth` holds the guard over the entire read/refresh/write or revoke sequence.
/// Every store instance addressing the same record must contend on the same lock,
/// even when used by different archives or processes. Dropping Guard releases it.
pub trait CredentialStore {
    type Error: Error + Send + Sync + 'static;
    type Guard;
    fn lock(&self, account: &str) -> Result<Self::Guard, Self::Error>;
    fn load(&self, account: &str) -> Result<Option<String>, Self::Error>;
    fn save(&self, account: &str, secret: &str) -> Result<(), Self::Error>;
    fn delete(&self, account: &str) -> Result<(), Self::Error>;
}

#[derive(Clone, Copy, Default)]
pub struct Keychain;

#[derive(Debug, thiserror::Error)]
pub enum KeychainError {
    #[cfg(target_os = "macos")]
    #[error("macOS Keychain operation failed")]
    Mac(#[from] security_framework::base::Error),
    #[error("stored credential is invalid UTF-8")]
    Utf8(#[from] std::string::FromUtf8Error),
    #[error("persistent account credentials currently require macOS Keychain")]
    Unsupported,
    #[error("credential lock file operation failed")]
    Io(#[from] std::io::Error),
}

#[cfg(any(target_os = "macos", test))]
fn lock_record(root: &Path, account: &str) -> std::io::Result<File> {
    std::fs::create_dir_all(root)?;
    let mut digest = Sha256::new();
    digest.update(SERVICE.as_bytes());
    digest.update([0]);
    digest.update(account.as_bytes());
    let hash: String = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let name = format!("{hash}.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join(name))?;
    file.try_lock_exclusive()?;
    // Never unlink lock files: a new inode would allow concurrent holders.
    Ok(file)
}

#[cfg(test)]
#[path = "credential_tests.rs"]
mod tests;

#[cfg(target_os = "macos")]
impl CredentialStore for Keychain {
    type Error = KeychainError;
    type Guard = File;
    fn lock(&self, account: &str) -> Result<Self::Guard, Self::Error> {
        let home = std::env::var_os("HOME")
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "HOME is unset"))?;
        lock_record(
            &std::path::PathBuf::from(home)
                .join("Library/Application Support/moonmail/credential-locks"),
            account,
        )
        .map_err(Into::into)
    }
    fn load(&self, account: &str) -> Result<Option<String>, Self::Error> {
        match security_framework::passwords::get_generic_password(SERVICE, account) {
            Ok(bytes) => Ok(Some(String::from_utf8(bytes)?)),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn save(&self, account: &str, secret: &str) -> Result<(), Self::Error> {
        Ok(security_framework::passwords::set_generic_password(
            SERVICE,
            account,
            secret.as_bytes(),
        )?)
    }
    fn delete(&self, account: &str) -> Result<(), Self::Error> {
        match security_framework::passwords::delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl CredentialStore for Keychain {
    type Error = KeychainError;
    type Guard = File;
    fn lock(&self, _account: &str) -> Result<Self::Guard, Self::Error> {
        Err(KeychainError::Unsupported)
    }
    fn load(&self, _account: &str) -> Result<Option<String>, Self::Error> {
        Err(KeychainError::Unsupported)
    }
    fn save(&self, _account: &str, _secret: &str) -> Result<(), Self::Error> {
        Err(KeychainError::Unsupported)
    }
    fn delete(&self, _account: &str) -> Result<(), Self::Error> {
        Err(KeychainError::Unsupported)
    }
}
