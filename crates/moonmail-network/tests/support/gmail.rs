use {
    moonmail_core::{BlobStore, MailboxRole, MessageId, MimeParser, ParsedMessage},
    moonmail_mime::StalwartParser,
    moonmail_network::{
        Account, LoginSecret, Provider, RustImap, SmtpSubmission, check_account,
        prepare_message_with_attachments, synchronize_cancellable, transmit_queued,
    },
    moonmail_store::{Archive, Draft, FileBlobs},
    rusqlite::{Connection, OpenFlags, params},
    std::time::{Duration, Instant},
};

pub type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

// Do not print provider replies, mailbox bodies, credentials, or account addresses
// on failure. CI logs contain phase labels and synthetic run IDs only.
pub fn stage<T, E>(label: &'static str, result: std::result::Result<T, E>) -> Result<T> {
    result.map_err(|_| label.into())
}

pub fn check(condition: bool, message: &'static str) -> Result {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn mailbox_key(email: &str) -> Result<String> {
    let (local, domain) = email.rsplit_once('@').ok_or("Invalid test email")?;
    check(
        matches!(
            domain.to_ascii_lowercase().as_str(),
            "gmail.com" | "googlemail.com"
        ),
        "Use dedicated consumer Gmail accounts",
    )?;
    check(
        !local.contains('+'),
        "Use the primary Gmail address, not a plus alias",
    )?;
    Ok(local.to_ascii_lowercase().replace('.', ""))
}

pub fn distinct_accounts(a: &str, b: &str) -> Result {
    check(
        mailbox_key(a)? != mailbox_key(b)?,
        "Two distinct Gmail mailboxes are required",
    )
}

pub struct Peer {
    pub account: Account,
    secret: LoginSecret,
    pub root: tempfile::TempDir,
}

pub struct Sent {
    pub rfc_id: String,
    pub parsed: ParsedMessage,
    pub outbox_id: i64,
}

impl Peer {
    pub fn load(email_name: &'static str, password_name: &'static str) -> Result<Self> {
        let email = stage(email_name, std::env::var(email_name))?;
        let password = stage(password_name, std::env::var(password_name))?;
        check(!password.trim().is_empty(), "Missing app-password secret")?;
        mailbox_key(&email)?;
        Ok(Self {
            account: stage(
                "validate Gmail test address",
                Account::preset(Provider::Google, &email),
            )?,
            secret: LoginSecret {
                bearer: false,
                value: password.into(),
            },
            root: stage("create disposable archive", tempfile::tempdir())?,
        })
    }

    pub fn check_connection(&self) -> Result {
        stage(
            "IMAP/SMTP authentication failed; check the dedicated account and app-password secrets",
            check_account(&self.account, &self.secret),
        )
    }

    pub fn send(&self, draft: &Draft, recipient: &Peer) -> Result<Sent> {
        let (raw, recipients) = stage(
            "prepare outgoing MIME",
            prepare_message_with_attachments(
                &self.account,
                draft,
                &FileBlobs::attachments(self.root.path()),
            ),
        )?;
        check(
            recipients.len() == 1
                && recipients[0] == recipient.account.email
                && draft.sender() == self.account.email,
            "Live test envelope must stay between the two configured accounts",
        )?;
        let parsed = stage("parse outgoing MIME", StalwartParser::default().parse(&raw))?;
        let rfc_id = parsed
            .message_id
            .clone()
            .ok_or("Outgoing MIME has no Message-ID")?;
        let mut archive = stage("open outgoing archive", Archive::open(self.root.path()))?;
        let blobs = FileBlobs::new(self.root.path());
        let hash = stage("freeze outgoing bytes", blobs.put(&raw))?;
        let outbox_id = stage(
            "queue outgoing MIME",
            archive.queue_with_message_id(
                draft,
                &self.account.email,
                &hash,
                &recipients,
                Some(&rfc_id),
            ),
        )?;
        stage(
            "freeze server-managed Sent policy",
            archive.prepare_sent_copy(outbox_id, false),
        )?;
        stage(
            "SMTP submission failed or uncertain; inspect test mailboxes before starting a new run",
            transmit_queued(
                &archive,
                &self.account.email,
                outbox_id,
                &blobs,
                &mut SmtpSubmission {
                    account: self.account.clone(),
                    secret: self.secret.clone(),
                },
            ),
        )?;
        Ok(Sent {
            rfc_id,
            parsed,
            outbox_id,
        })
    }

    pub fn sync(&self, deadline: Instant) -> Result {
        check(Instant::now() < deadline, "Live Gmail deadline exceeded")?;
        let mut incoming = stage(
            "reconnect IMAP",
            RustImap::connect(&self.account, &self.secret),
        )?;
        let mut archive = stage("reopen local archive", Archive::open(self.root.path()))?;
        stage(
            "IMAP synchronization failed or timed out",
            synchronize_cancellable(
                &mut incoming,
                &self.account,
                &mut archive,
                &FileBlobs::new(self.root.path()),
                |_| {},
                || Instant::now() >= deadline,
            ),
        )
    }

    pub fn find(&self, sent: &Sent, role: MailboxRole) -> Result<Option<MessageId>> {
        let archive = stage("open observed archive", Archive::open(self.root.path()))?;
        let folder = stage(
            "Gmail omitted an unambiguous Inbox/Sent role",
            archive.role_mailbox(&self.account.email, role),
        )?;
        let db = stage(
            "open read-only observation",
            Connection::open_with_flags(
                self.root.path().join("archive.sqlite3"),
                OpenFlags::SQLITE_OPEN_READ_ONLY,
            ),
        )?;
        let mut query = stage("prepare Message-ID observation", db.prepare("SELECT m.id FROM messages m JOIN occurrences o ON o.message_id=m.id WHERE m.account=?1 AND m.rfc_id=?2 AND o.mailbox_id=?3 AND o.uid IS NOT NULL LIMIT 2"))?;
        let rows = stage(
            "query exact Message-ID",
            query.query_map(params![self.account.email, sent.rfc_id, folder], |row| {
                row.get::<_, i64>(0)
            }),
        )?;
        let ids = stage(
            "read Message-ID matches",
            rows.collect::<rusqlite::Result<Vec<_>>>(),
        )?;
        check(
            ids.len() <= 1,
            "Duplicate Message-ID copies observed in test folder",
        )?;
        Ok(stage(
            "Invalid observed message ID",
            ids.first().map(|id| u64::try_from(*id)).transpose(),
        )?
        .map(MessageId))
    }

    pub fn verify(&self, id: MessageId, sent: &Sent) -> Result {
        let archive = stage("open received archive", Archive::open(self.root.path()))?;
        let hash = stage("read received blob reference", archive.message_blob(id))?;
        let raw = stage(
            "load received MIME",
            FileBlobs::new(self.root.path()).get(&hash),
        )?;
        let parsed = stage("parse received MIME", StalwartParser::default().parse(&raw))?;
        verify_parsed(&parsed, &sent.parsed)
    }

    pub fn wait_for(&self, sent: &Sent, role: MailboxRole, deadline: Instant) -> Result<MessageId> {
        loop {
            self.sync(deadline)?;
            if let Some(id) = self.find(sent, role)? {
                self.verify(id, sent)?;
                return Ok(id);
            }
            check(
                Instant::now() < deadline,
                "Message not observed before deadline; inspect Spam and test run subject",
            )?;
            std::thread::sleep(
                Duration::from_secs(5).min(deadline.saturating_duration_since(Instant::now())),
            );
        }
    }
}

pub fn verify_parsed(actual: &ParsedMessage, expected: &ParsedMessage) -> Result {
    check(
        actual.message_id == expected.message_id,
        "Message-ID changed",
    )?;
    check(
        actual.from == expected.from
            && actual.to == expected.to
            && actual.reply_to == expected.reply_to
            && actual.cc == expected.cc,
        "Received sender/recipient mismatch",
    )?;
    check(
        actual.subject == expected.subject && actual.text_body == expected.text_body,
        "Received subject/body mismatch",
    )?;
    check(
        actual.in_reply_to == expected.in_reply_to && actual.references == expected.references,
        "Received reply ancestry mismatch",
    )
}
