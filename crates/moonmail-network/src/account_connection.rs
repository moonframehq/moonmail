use crate::{Account, LoginSecret, Result, RustImap, SmtpSubmission};

/// Verify both authenticated services without downloading mail or sending a
/// message. The caller runs this on a connection-owning worker.
pub fn check_account(account: &Account, secret: &LoginSecret) -> Result<()> {
    account.validate()?;
    tracing::debug!(provider = ?account.provider, transport = ?account.transport, "checking incoming mail connection");
    if account.transport == crate::Transport::Jmap {
        crate::jmap::Client::fastmail(secret.value.clone())?.check_identity(&account.email)?;
        return Ok(());
    }
    let incoming = RustImap::connect(account, secret)?;
    drop(incoming);
    tracing::debug!("IMAP check passed, checking SMTP connection");
    SmtpSubmission {
        account: account.clone(),
        secret: secret.clone(),
    }
    .check_connection()?;
    tracing::debug!("SMTP check passed");
    Ok(())
}
