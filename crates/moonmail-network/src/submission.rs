use {
    crate::{
        Account, Error, LoginSecret, Provider, Result, TlsMode, oauth_bearer, validate_address,
    },
    moonmail_core::BlobStore,
    moonmail_store::Draft,
    std::{collections::HashSet, time::Duration},
};

#[derive(Debug, thiserror::Error)]
pub enum SubmitError {
    #[error("Invalid SMTP account configuration")]
    Account(#[source] Box<Error>),
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

    /// The conservative default forbids retries. Override only when the adapter
    /// can prove no recipient submission began (including no partial delivery).
    fn failure(_error: &Self::Error) -> SubmissionFailure {
        SubmissionFailure::Unknown
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmissionFailure {
    NotSubmitted,
    Unknown,
}

pub struct SmtpSubmission {
    pub account: Account,
    pub secret: LoginSecret,
}
impl Submission for SmtpSubmission {
    type Error = SubmitError;
    fn failure(error: &SubmitError) -> SubmissionFailure {
        if matches!(error, SubmitError::Unknown(_)) {
            SubmissionFailure::Unknown
        } else {
            SubmissionFailure::NotSubmitted
        }
    }
    fn submit(
        &mut self,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), SubmitError> {
        self.submit_using(self.builder(None)?, sender, recipients, raw)
    }
}

impl SmtpSubmission {
    /// Submit once using caller-supplied TLS trust, preserving normal authentication.
    pub fn submit_with_tls(
        &self,
        connector: tokio_rustls::TlsConnector,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), SubmitError> {
        self.submit_using(self.builder(Some(connector))?, sender, recipients, raw)
    }

    fn submit_using(
        &self,
        builder: mail_send::SmtpClientBuilder<String>,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), SubmitError> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let mut client = builder.connect().await.map_err(SubmitError::Connect)?;
                self.authenticate_external(&mut client)
                    .await
                    .map_err(SubmitError::Connect)?;
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

impl SmtpSubmission {
    /// Authenticate without issuing MAIL FROM, RCPT TO or DATA.
    pub fn check_connection(&self) -> std::result::Result<(), SubmitError> {
        self.check_using(self.builder(None)?)
    }

    /// Authenticate with explicit trust roots without submitting a message.
    pub fn check_connection_with_tls(
        &self,
        connector: tokio_rustls::TlsConnector,
    ) -> std::result::Result<(), SubmitError> {
        self.check_using(self.builder(Some(connector))?)
    }

    fn check_using(
        &self,
        builder: mail_send::SmtpClientBuilder<String>,
    ) -> std::result::Result<(), SubmitError> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(async {
                let mut client = builder.connect().await.map_err(SubmitError::Connect)?;
                self.authenticate_external(&mut client)
                    .await
                    .map_err(SubmitError::Connect)?;
                Ok(())
            })
    }

    fn builder(
        &self,
        connector: Option<tokio_rustls::TlsConnector>,
    ) -> std::result::Result<mail_send::SmtpClientBuilder<String>, SubmitError> {
        self.account
            .validate()
            .map_err(|error| SubmitError::Account(Box::new(error)))?;
        let credentials = if self.secret.bearer
            && matches!(
                self.account.provider,
                Provider::Google | Provider::Microsoft
            ) {
            mail_send::Credentials::new_xoauth2(
                self.account.smtp_login().to_owned(),
                self.secret.value.clone(),
            )
        } else if self.secret.bearer {
            mail_send::Credentials::new_oauth(oauth_bearer(
                &self.account.smtp_host,
                self.account.smtp_port,
                &self.secret.value,
            ))
        } else {
            mail_send::Credentials::new(
                self.account.smtp_login().to_owned(),
                self.secret.value.clone(),
            )
        };
        let builder = if let Some(connector) = connector {
            mail_send::SmtpClientBuilder::new_with_tls_connector(
                self.account.smtp_host.clone(),
                self.account.smtp_port,
                connector,
            )
        } else {
            mail_send::SmtpClientBuilder::new(
                self.account.smtp_host.clone(),
                self.account.smtp_port,
            )
            .map_err(SubmitError::Configuration)?
        }
        .implicit_tls(self.account.smtp_tls == TlsMode::Implicit)
        .timeout(Duration::from_secs(30));
        Ok(if self.external_oauth() {
            builder
        } else {
            builder.credentials(credentials)
        })
    }

    fn external_oauth(&self) -> bool {
        self.secret.bearer
            && matches!(
                self.account.provider,
                Provider::Google | Provider::Microsoft
            )
    }

    async fn authenticate_external<T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(
        &self,
        client: &mut mail_send::SmtpClient<T>,
    ) -> std::result::Result<(), mail_send::Error> {
        if !self.external_oauth() {
            return Ok(());
        }
        use base64::Engine;
        let payload = format!(
            "user={}\x01auth=Bearer {}\x01\x01",
            self.account.smtp_login(),
            self.secret.value
        );
        let reply = client
            .cmd(format!(
                "AUTH XOAUTH2 {}\r\n",
                base64::engine::general_purpose::STANDARD.encode(payload)
            ))
            .await?;
        if reply.code() == 235 {
            return Ok(());
        }
        // Gmail's JSON failure challenge requires an empty response. The
        // pinned mail-send adapter otherwise repeats the initial credential.
        let reply = if reply.code() == 334 {
            client.cmd(b"\r\n").await?
        } else {
            reply
        };
        Err(mail_send::Error::AuthenticationFailed(reply))
    }
}

#[cfg(test)]
mod oauth_tests {
    use {
        super::*,
        base64::Engine,
        std::io::{BufRead, BufReader, Write},
    };

    #[test]
    fn xoauth2_smtp_succeeds_or_terminates_error_challenge_without_resending_credentials()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for success in [true, false] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
            let address = listener.local_addr()?;
            let server = std::thread::spawn(move || -> std::io::Result<()> {
                let (mut socket, _) = listener.accept()?;
                socket.set_read_timeout(Some(Duration::from_secs(5)))?;
                let mut reader = BufReader::new(socket.try_clone()?);
                let mut command = String::new();
                reader.read_line(&mut command)?;
                let payload = command
                    .trim_end()
                    .strip_prefix("AUTH XOAUTH2 ")
                    .ok_or_else(|| std::io::Error::other("wrong mechanism"))?;
                assert_eq!(
                    base64::engine::general_purpose::STANDARD
                        .decode(payload)
                        .map_err(std::io::Error::other)?,
                    b"user=me@example.org\x01auth=Bearer synthetic\x01\x01"
                );
                if success {
                    socket.write_all(b"235 2.7.0 accepted\r\n")?;
                } else {
                    socket.write_all(b"334 eyJzdGF0dXMiOiI0MDEifQ==\r\n")?;
                    command.clear();
                    reader.read_line(&mut command)?;
                    assert_eq!(command, "\r\n");
                    socket.write_all(b"535 5.7.1 authentication failed\r\n")?;
                }
                Ok(())
            });
            let smtp = SmtpSubmission {
                account: Account::preset(Provider::Google, "me@example.org")?,
                secret: LoginSecret {
                    bearer: true,
                    value: "synthetic".into(),
                },
            };
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(async {
                    let mut client =
                        mail_send::SmtpClient::connect(address, Duration::from_secs(5)).await?;
                    smtp.authenticate_external(&mut client).await
                });
            assert_eq!(result.is_ok(), success);
            server.join().map_err(|_| "fake SMTP server panicked")??;
        }
        Ok(())
    }
}

/// Frozen RFC 822 bytes and their SMTP envelope recipients.
pub type PreparedMessage = (Vec<u8>, Vec<String>);

pub fn prepare_message(account: &Account, draft: &Draft) -> Result<PreparedMessage> {
    if !draft.attachments.is_empty() {
        return Err(Error::Invalid(
            "Attachment blobs are required to prepare this draft",
        ));
    }
    build_message(account, draft, &[])
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareError<E: std::error::Error + 'static> {
    #[error("could not load an immutable draft attachment")]
    Attachment(#[source] E),
    #[error(transparent)]
    Message(#[from] Error),
}

/// All attachment bytes are verified before freezing MIME or queuing delivery.
/// Missing/corrupt blobs fail the whole preparation, never silently omit files.
pub fn prepare_message_with_attachments<B: BlobStore>(
    account: &Account,
    draft: &Draft,
    blobs: &B,
) -> std::result::Result<PreparedMessage, PrepareError<B::Error>> {
    draft.validate_attachments().map_err(Error::from)?;
    let mut loaded = Vec::with_capacity(draft.attachments.len());
    for attachment in &draft.attachments {
        let bytes = blobs
            .get(&attachment.blob)
            .map_err(PrepareError::Attachment)?;
        if bytes.len() as u64 != attachment.bytes {
            return Err(Error::Invalid("Attachment size does not match the saved draft").into());
        }
        loaded.push(bytes);
    }
    Ok(build_message(account, draft, &loaded)?)
}

fn build_message(
    account: &Account,
    draft: &Draft,
    attachments: &[Vec<u8>],
) -> Result<PreparedMessage> {
    use crate::recipients::{header_addresses, parse_recipients, unique};
    validate_address(&account.email)?;
    draft.validate_identity()?;
    if !draft.account.is_empty() && draft.account != account.email {
        return Err(Error::Invalid(
            "Connect the draft's original account before sending",
        ));
    }
    let mut seen = HashSet::new();
    let to = unique(parse_recipients(&draft.recipients)?, &mut seen);
    let cc = unique(parse_recipients(&draft.cc)?, &mut seen);
    let bcc = unique(parse_recipients(&draft.bcc)?, &mut seen);
    let recipients: Vec<_> = to
        .iter()
        .chain(&cc)
        .chain(&bcc)
        .map(|address| address.address.clone())
        .collect();
    if recipients.is_empty() || recipients.len() > 100 {
        return Err(Error::Invalid("Enter between one and 100 recipients"));
    }
    if draft.subject.chars().any(char::is_control) {
        return Err(Error::Invalid("Subject cannot contain control characters"));
    }
    let mut builder = mail_builder::MessageBuilder::new()
        .subject(draft.subject.as_str())
        .message_id(format!("{}@moonmail.local", uuid::Uuid::new_v4()))
        .text_body(draft.body.as_str());
    if let Some(identity) = &draft.identity {
        crate::validate_identity(identity)?;
        builder = builder.from(mail_builder::headers::address::Address::new_address(
            if identity.name.is_empty() {
                None
            } else {
                Some(identity.name.as_str())
            },
            identity.address.as_str(),
        ));
        if !identity.reply_to.is_empty() {
            builder = builder.reply_to(identity.reply_to.as_str());
        }
    } else {
        builder = builder.from(account.email.as_str());
    }
    builder = if to.is_empty() {
        builder.to(mail_builder::headers::address::Address::new_group(
            Some("Undisclosed recipients"),
            Vec::new(),
        ))
    } else {
        builder.to(header_addresses(&to))
    };
    if !cc.is_empty() {
        builder = builder.cc(header_addresses(&cc));
    }
    for (attachment, bytes) in draft.attachments.iter().zip(attachments) {
        builder = builder.attachment(
            "application/octet-stream",
            attachment.name.as_str(),
            bytes.as_slice(),
        );
    }
    // Bcc exists solely in the durable envelope; never attach it to the builder.
    let references = reply_references(draft)?;
    if !draft.reply_id.is_empty() {
        builder = builder.in_reply_to(draft.reply_id.as_str());
    }
    if !references.is_empty() {
        builder = builder.references(references);
    }
    Ok((builder.write_to_vec()?, recipients))
}

/// Keep the original root and the newest ancestors, then append the immediate
/// parent exactly once. Bound outgoing headers to 100 IDs; malformed historical
/// IDs are omitted, but an invalid immediate parent is an explicit error.
fn reply_references(draft: &Draft) -> Result<Vec<&str>> {
    fn valid(id: &str) -> bool {
        !id.is_empty()
            && id.len() <= 995
            && id.contains('@')
            && id
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'<' | b'>'))
    }
    if !draft.reply_id.is_empty() && !valid(&draft.reply_id) {
        return Err(Error::Invalid("Reply Message-ID is malformed"));
    }
    let mut references = Vec::with_capacity(100);
    for id in draft.references.iter().map(String::as_str) {
        if !valid(id) || id == draft.reply_id || references.contains(&id) {
            continue;
        }
        if references.len() == 99 {
            references.remove(1); // Preserve the root while dropping old ancestors.
        }
        references.push(id);
    }
    if !draft.reply_id.is_empty() {
        references.push(&draft.reply_id);
    }
    Ok(references)
}
