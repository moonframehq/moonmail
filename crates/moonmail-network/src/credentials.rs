use std::error::Error;

pub trait CredentialStore {
    type Error: Error + Send + Sync + 'static;
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
}

#[cfg(target_os = "macos")]
impl CredentialStore for Keychain {
    type Error = KeychainError;
    fn load(&self, account: &str) -> Result<Option<String>, Self::Error> {
        match security_framework::passwords::get_generic_password("dev.moonmail.account", account) {
            Ok(bytes) => Ok(Some(String::from_utf8(bytes)?)),
            Err(error) if error.code() == -25300 => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
    fn save(&self, account: &str, secret: &str) -> Result<(), Self::Error> {
        Ok(security_framework::passwords::set_generic_password(
            "dev.moonmail.account",
            account,
            secret.as_bytes(),
        )?)
    }
    fn delete(&self, account: &str) -> Result<(), Self::Error> {
        match security_framework::passwords::delete_generic_password(
            "dev.moonmail.account",
            account,
        ) {
            Ok(()) => Ok(()),
            Err(error) if error.code() == -25300 => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(not(target_os = "macos"))]
impl CredentialStore for Keychain {
    type Error = KeychainError;
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
