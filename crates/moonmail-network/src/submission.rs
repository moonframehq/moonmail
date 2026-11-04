use {
    crate::{Account, Auth, Error, Keychain, LoginSecret, Result, oauth_bearer, validate_address},
    moonmail_core::BlobStore,
    moonmail_store::{Archive, Draft, FileBlobs},
    std::{path::Path, time::Duration},
};

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("SMTP connection/authentication failed before submission")]
    Connect(#[source] mail_send::Error),
    #[error("SMTP delivery outcome is uncertain; check Sent before composing a replacement")]
    Unknown(#[source] mail_send::Error),
    #[error("SMTP TLS configuration failed: {0}")]
    Configuration(String),
    #[error("SMTP runtime failed")]
    Runtime(#[from] std::io::Error),
}

/// Frozen bytes and envelope, one attempt only. An error after submission starts
/// is conservatively unknown and must never trigger an automatic retry.
pub trait Submission {
    type Error: std::error::Error + Send + Sync + 'static;
    fn submit(
        &mut self,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), Self::Error>;
}

pub struct SmtpSubmission {
    pub account: Account,
    pub secret: LoginSecret,
}
impl Submission for SmtpSubmission {
    type Error = SubmitError;
    fn submit(
        &mut self,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), SubmitError> {
        let credentials = if self.secret.bearer {
            mail_send::Credentials::new_oauth(oauth_bearer(
                &self.account.smtp_host,
                self.account.smtp_port,
                &self.secret.value,
            ))
        } else {
            mail_send::Credentials::new(self.account.email.clone(), self.secret.value.clone())
        };
        let builder = mail_send::SmtpClientBuilder::new(
            self.account.smtp_host.clone(),
            self.account.smtp_port,
        )
        .map_err(SubmitError::Configuration)?
        .implicit_tls(true)
        .timeout(Duration::from_secs(30))
        .credentials(credentials);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let mut client = builder.connect().await.map_err(SubmitError::Connect)?;
                client
                    .send(mail_send::smtp::message::Message::new(
                        sender,
                        recipients.iter().map(String::as_str),
                        raw,
                    ))
                    .await
                    .map_err(SubmitError::Unknown)?;
                Ok(())
            })
    }
}

pub fn prepare_message(account: &Account, draft: &Draft) -> Result<(Vec<u8>, Vec<String>)> {
    validate_address(&account.email)?;
    let recipients: Vec<String> = draft
        .recipients
        .split(',')
        .map(str::trim)
        .map(str::to_owned)
        .collect();
    if recipients.is_empty() || recipients.len() > 100 {
        return Err(Error::Invalid("Enter between one and 100 recipients"));
    }
    for recipient in &recipients {
        validate_address(recipient)?;
    }
    if draft.subject.contains(['\r', '\n']) {
        return Err(Error::Invalid("Subject must be one line"));
    }
    let mut builder = mail_builder::MessageBuilder::new()
        .from(account.email.as_str())
        .to(recipients.iter().map(String::as_str).collect::<Vec<_>>())
        .subject(draft.subject.as_str())
        .message_id(format!("{}@moonmail.local", uuid::Uuid::new_v4()))
        .text_body(draft.body.as_str());
    if !draft.reply_id.is_empty() {
        builder = builder
            .in_reply_to(draft.reply_id.as_str())
            .references(draft.reply_id.as_str());
    }
    Ok((builder.write_to_vec()?, recipients))
}

pub fn send_queued(root: &Path, id: i64) -> Result<()> {
    let account = Account::load(root)?.ok_or(Error::Invalid("Connect an account first"))?;
    let secret = Auth::new(Keychain, root).credentials(&account.email)?;
    let archive = Archive::open(root)?;
    let item = archive
        .outbox()?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or(Error::Invalid("Outbox item not found"))?;
    if item.account != account.email {
        return Err(Error::Invalid("Outbox belongs to another account"));
    }
    let raw = FileBlobs::new(root).get(&item.blob)?;
    if !archive.claim_send(id)? {
        return Err(Error::Invalid(
            "This outbox item has already been attempted",
        ));
    }
    match (SmtpSubmission { account, secret }).submit(&item.sender, &item.recipients, &raw) {
        Ok(()) => archive.send_result(id, "sent", "Accepted by SMTP server")?,
        Err(error) => {
            let state = if matches!(error, SubmitError::Unknown(_)) {
                "unknown"
            } else {
                "failed"
            };
            archive.send_result(id, state, &error.to_string())?;
            return Err(Error::Invalid(
                "Submission did not complete; inspect Outbox status",
            ));
        },
    }
    Ok(())
}
