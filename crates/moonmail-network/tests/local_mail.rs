//! Real adapters against the native fixture or the same daemon running in Docker.
use {
    moonmail_core::{BlobStore, MailRepository, MailboxRole, MimeAttachments, MimeParser},
    moonmail_mime::{MailparseParser, StalwartParser},
    moonmail_network::{
        Account, IncomingMail, LoginSecret, Provider, ReplyMode, RustImap, SmtpSubmission,
        Submission, SubmissionFailure, SubmitError, TlsMode, compose_reply,
        prepare_message_with_attachments, synchronize_cancellable, transmit_queued,
    },
    moonmail_store::{Archive, Draft, DraftAttachment, FileBlobs},
    moonmail_test_maild::{ACCOUNTS, Config, Endpoints, PASSWORD, Server},
    std::{path::PathBuf, sync::Arc},
};

#[path = "support/stalwart_mail.rs"]
mod stalwart_mail;

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Fixture {
    _server: Option<Server>,
    endpoints: Endpoints,
    stalwart_credentials: Option<[LoginSecret; 2]>,
    imap: native_tls::TlsConnector,
    smtp: tokio_rustls::TlsConnector,
}

impl Fixture {
    fn new(external: bool, server_sent: bool) -> Result<Self> {
        let (server, endpoints, ca) =
            if external && let Some(path) = std::env::var_os("MOONMAIL_TEST_MAILD_STATE") {
                let path = PathBuf::from(path);
                (
                    None,
                    serde_json::from_slice(&std::fs::read(path.join("ready.json"))?)?,
                    std::fs::read(path.join("ca.der"))?,
                )
            } else {
                let server = Server::start(Config {
                    server_sent,
                    ..Default::default()
                })?;
                let endpoints = server.endpoints.clone();
                let ca = server.ca_der.clone();
                (Some(server), endpoints, ca)
            };
        Self::trusted(server, endpoints, ca, None)
    }

    fn stalwart(bearer: bool) -> Result<Self> {
        let path = PathBuf::from(
            std::env::var_os("MOONMAIL_STALWART_STATE")
                .ok_or("Run just test-stalwart-imap-smtp")?,
        );
        let credentials = |user| -> Result<LoginSecret> {
            let kind = if bearer { "TOKEN" } else { "PASSWORD" };
            Ok(LoginSecret {
                value: std::env::var(format!("MOONMAIL_STALWART_{user}_{kind}"))?.into(),
                bearer,
            })
        };
        Self::trusted(
            None,
            serde_json::from_slice(&std::fs::read(path.join("ready.json"))?)?,
            std::fs::read(path.join("ca.der"))?,
            Some([credentials("ALICE")?, credentials("BOB")?]),
        )
    }

    fn trusted(
        server: Option<Server>,
        endpoints: Endpoints,
        ca: Vec<u8>,
        stalwart_credentials: Option<[LoginSecret; 2]>,
    ) -> Result<Self> {
        let imap = native_tls::TlsConnector::builder()
            .disable_built_in_roots(true)
            .add_root_certificate(native_tls::Certificate::from_der(&ca)?)
            .build()?;
        let mut roots = rustls::RootCertStore::empty();
        roots.add(rustls::pki_types::CertificateDer::from(ca))?;
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_root_certificates(roots)
        .with_no_client_auth();
        Ok(Self {
            _server: server,
            endpoints,
            stalwart_credentials,
            imap,
            smtp: Arc::new(config).into(),
        })
    }

    fn peer(&self, index: usize, mode: TlsMode) -> Result<Peer> {
        let (address, secret) = match &self.stalwart_credentials {
            Some(credentials) => (
                ["alice@moonmail.example.org", "bob@moonmail.example.org"][index],
                credentials[index].clone(),
            ),
            None => (ACCOUNTS[index], secret()),
        };
        let mut account = Account::preset(Provider::Other, address)?;
        account.imap_host = "localhost".into();
        account.smtp_host = "localhost".into();
        account.imap_tls = mode;
        account.smtp_tls = mode;
        (account.imap_port, account.smtp_port) = if mode == TlsMode::Implicit {
            (self.endpoints.imap_tls, self.endpoints.smtp_tls)
        } else {
            (self.endpoints.imap_starttls, self.endpoints.smtp_starttls)
        };
        Ok(Peer {
            account,
            secret,
            root: tempfile::tempdir()?,
        })
    }

    fn incoming(&self, peer: &Peer) -> Result<RustImap> {
        Ok(RustImap::connect_with_tls(
            &peer.account,
            &peer.secret,
            &self.imap,
        )?)
    }

    fn session(
        &self,
        peer: &Peer,
    ) -> Result<imap::Session<native_tls::TlsStream<std::net::TcpStream>>> {
        let socket = std::net::TcpStream::connect(("127.0.0.1", peer.account.imap_port))?;
        socket.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        let mut client = imap::Client::new(self.imap.connect("localhost", socket)?);
        client.read_greeting()?;
        Ok(client
            .login(&peer.account.email, PASSWORD)
            .map_err(|(e, _)| e)?)
    }

    fn sync(&self, peer: &Peer) -> Result {
        synchronize_cancellable(
            &mut self.incoming(peer)?,
            &peer.account,
            &mut Archive::open(peer.root.path())?,
            &FileBlobs::new(peer.root.path()),
            |_| {},
            || false,
        )?;
        Ok(())
    }
}

fn secret() -> LoginSecret {
    LoginSecret {
        bearer: false,
        value: PASSWORD.into(),
    }
}
struct Peer {
    account: Account,
    secret: LoginSecret,
    root: tempfile::TempDir,
}
struct TrustedSubmission<'a> {
    smtp: SmtpSubmission,
    fixture: &'a Fixture,
}
impl Submission for TrustedSubmission<'_> {
    type Error = SubmitError;
    fn submit(
        &mut self,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), SubmitError> {
        self.smtp
            .submit_with_tls(self.fixture.smtp.clone(), sender, recipients, raw)
    }
    fn failure(error: &SubmitError) -> SubmissionFailure {
        SmtpSubmission::failure(error)
    }
}

impl Peer {
    fn send(&self, fixture: &Fixture, draft: &Draft, server_sent: bool) -> Result<(i64, Vec<u8>)> {
        let (raw, recipients) = prepare_message_with_attachments(
            &self.account,
            draft,
            &FileBlobs::attachments(self.root.path()),
        )?;
        let parsed = StalwartParser::default().parse(&raw)?;
        let mut archive = Archive::open(self.root.path())?;
        let blobs = FileBlobs::new(self.root.path());
        let hash = blobs.put(&raw)?;
        let id = archive.queue_with_message_id(
            draft,
            &self.account.email,
            &hash,
            &recipients,
            parsed.message_id.as_deref(),
        )?;
        archive.prepare_sent_copy(id, !server_sent)?;
        transmit_queued(
            &archive,
            &self.account.email,
            id,
            &blobs,
            &mut TrustedSubmission {
                smtp: SmtpSubmission {
                    account: self.account.clone(),
                    secret: self.secret.clone(),
                },
                fixture,
            },
        )?;
        if !server_sent {
            moonmail_network::save_sent_copy(
                &mut fixture.incoming(self)?,
                &self.account,
                &archive,
                &blobs,
                id,
            )?;
        }
        Ok((id, raw))
    }

    fn observe(&self, raw: &[u8], role: MailboxRole) -> Result<moonmail_core::MessageId> {
        self.find_observed(raw, role, false)?
            .ok_or_else(|| "Missing remote copy".into())
    }

    fn find_observed(
        &self,
        raw: &[u8],
        role: MailboxRole,
        smtp_trace: bool,
    ) -> Result<Option<moonmail_core::MessageId>> {
        let archive = Archive::open(self.root.path())?;
        let folder = archive.role_mailbox(&self.account.email, role)?;
        let rfc_id = StalwartParser::default()
            .parse(raw)?
            .message_id
            .ok_or("missing Message-ID")?;
        let db = rusqlite::Connection::open_with_flags(
            self.root.path().join("archive.sqlite3"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let mut query = db.prepare("SELECT m.id FROM messages m JOIN occurrences o ON o.message_id=m.id WHERE m.account=?1 AND m.rfc_id=?2 AND o.mailbox_id=?3 AND o.uid IS NOT NULL")?;
        let ids = query
            .query_map(rusqlite::params![self.account.email, rfc_id, folder], |r| {
                r.get::<_, i64>(0)
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if ids.is_empty() {
            return Ok(None);
        }
        assert_eq!(ids.len(), 1, "expected exactly one observed remote copy");
        let id = moonmail_core::MessageId(u64::try_from(ids[0])?);
        let actual = FileBlobs::new(self.root.path()).get(&archive.message_blob(id)?)?;
        // SMTP relays prepend transport headers. The frozen MIME must still be
        // byte-identical, including its headers, attachments, and dot-stuffed body.
        let actual = if smtp_trace && role == MailboxRole::Inbox {
            let start = actual
                .windows(raw.len())
                .position(|w| w == raw)
                .ok_or("Delivered MIME differs from frozen message")?;
            assert!(
                !actual[..start].windows(4).any(|w| w == b"\r\n\r\n"),
                "Transport headers ended before frozen MIME"
            );
            &actual[start..]
        } else {
            actual.as_slice()
        };
        // mail-send terminates DATA with an additional CRLF before the dot.
        assert!(
            actual == raw || actual == [raw, b"\r\n"].concat(),
            "received MIME bytes changed beyond SMTP framing"
        );
        Ok(Some(id))
    }
}

fn await_copy(
    fixture: &Fixture,
    peer: &Peer,
    raw: &[u8],
    role: MailboxRole,
) -> Result<moonmail_core::MessageId> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        fixture.sync(peer)?;
        if let Some(id) = peer.find_observed(raw, role, fixture.stalwart_credentials.is_some())? {
            return Ok(id);
        }
        if std::time::Instant::now() >= until {
            return Err("IMAP delivery deadline expired".into());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

fn round_trip(fixture: &Fixture, mode: TlsMode, server_sent: bool) -> Result {
    let a = fixture.peer(0, mode)?;
    let b = fixture.peer(1, mode)?;
    for peer in [&a, &b] {
        SmtpSubmission {
            account: peer.account.clone(),
            secret: peer.secret.clone(),
        }
        .check_connection_with_tls(fixture.smtp.clone())?;
        fixture.sync(peer)?;
    }
    let bytes = b"synthetic attachment\n\0\xff";
    let draft = Draft {
        id: uuid::Uuid::new_v4().to_string(),
        account: a.account.email.clone(),
        recipients: b.account.email.clone(),
        subject: format!("Local round trip {}", uuid::Uuid::new_v4()),
        body: "Unicode café 🌙\n.dot-stuffed line\n".into(),
        attachments: vec![DraftAttachment {
            id: "fixture".into(),
            name: "fixture.bin".into(),
            blob: FileBlobs::attachments(a.root.path()).put(bytes)?,
            bytes: bytes.len() as u64,
        }],
        ..Default::default()
    };
    let watcher = if fixture.stalwart_credentials.is_some() {
        Some(stalwart_mail::watch(fixture, &b)?)
    } else {
        None
    };
    let (a_outbox, raw) = a.send(fixture, &draft, server_sent)?;
    let received = if let Some((worker, checkpoint)) = watcher {
        stalwart_mail::pushed(worker, checkpoint, &b, &raw)?
    } else {
        await_copy(fixture, &b, &raw, MailboxRole::Inbox)?
    };
    let attachments = MailparseParser.attachments(&raw)?;
    assert_eq!(attachments.len(), 1);
    assert_eq!(MailparseParser.attachment(&raw, &attachments[0].id)?, bytes);
    let archive = Archive::open(b.root.path())?;
    let mut reply = compose_reply(
        &archive.detail(received)?,
        archive.reply_headers(
            received,
            &StalwartParser::default(),
            &FileBlobs::new(b.root.path()),
        )?,
        &b.account.email,
        ReplyMode::Sender,
    )?;
    reply.id = uuid::Uuid::new_v4().to_string();
    reply.body = format!("Reply from Bob\n{}", reply.body);
    let (b_outbox, response) = b.send(fixture, &reply, server_sent)?;
    let parsed = StalwartParser::default().parse(&response)?;
    let original = StalwartParser::default()
        .parse(&raw)?
        .message_id
        .ok_or("missing Message-ID")?;
    assert_eq!(parsed.in_reply_to.as_deref(), Some(original.as_str()));
    assert!(parsed.references.contains(&original));
    // Reconnect and reopen archives, including a second scan after Sent APPEND.
    for _ in 0..2 {
        fixture.sync(&a)?;
        fixture.sync(&b)?;
    }
    await_copy(fixture, &a, &response, MailboxRole::Inbox)?;
    await_copy(fixture, &b, &raw, MailboxRole::Inbox)?;
    a.observe(&raw, MailboxRole::Sent)?;
    b.observe(&response, MailboxRole::Sent)?;
    for (peer, id) in [(&a, a_outbox), (&b, b_outbox)] {
        let item = Archive::open(peer.root.path())?.outbox_item(id)?;
        assert_eq!(item.state, "sent");
        assert_eq!(item.sent_status, "visible");
        assert_eq!(
            item.sent_copy_state,
            if server_sent { "server" } else { "saved" }
        );
    }
    if fixture.stalwart_credentials.is_some() {
        stalwart_mail::verify_edits(fixture, &a, &b, received)?;
    }
    Ok(())
}

#[test]
#[ignore = "requires an independent loopback Stalwart, run just test-stalwart-imap-smtp"]
fn stalwart_imap_smtp() -> Result {
    for bearer in [false, true] {
        let fixture = Fixture::stalwart(bearer)?;
        for mode in [TlsMode::Implicit, TlsMode::Starttls] {
            round_trip(&fixture, mode, false)?;
            println!(
                "Verified bidirectional delivery, IDLE and mailbox edits over {mode:?} with {}",
                if bearer { "OAuth" } else { "passwords" }
            );
        }
    }
    Ok(())
}

#[test]
fn local_round_trip() -> Result {
    let fixture = Fixture::new(true, false)?;
    for mode in [TlsMode::Implicit, TlsMode::Starttls] {
        round_trip(&fixture, mode, false)?;
    }
    Ok(())
}

#[test]
fn server_managed_sent_round_trip() -> Result {
    round_trip(&Fixture::new(false, true)?, TlsMode::Implicit, true)
}

#[test]
fn trust_authentication_isolation_and_uid_operations() -> Result {
    let fixture = Fixture::new(false, false)?;
    let a = fixture.peer(0, TlsMode::Implicit)?;
    let b = fixture.peer(1, TlsMode::Starttls)?;
    assert!(
        RustImap::connect(&a.account, &secret()).is_err(),
        "untrusted fixture CA accepted"
    );
    let unrelated = Fixture::new(false, false)?;
    assert!(RustImap::connect_with_tls(&a.account, &secret(), &unrelated.imap).is_err());
    let mut bad = secret();
    bad.value = "wrong".into();
    assert!(RustImap::connect_with_tls(&a.account, &bad, &fixture.imap).is_err());
    let smtp = SmtpSubmission {
        account: a.account.clone(),
        secret: secret(),
    };
    assert!(
        smtp.check_connection_with_tls(unrelated.smtp.clone())
            .is_err()
    );
    assert!(
        SmtpSubmission {
            account: a.account.clone(),
            secret: bad
        }
        .check_connection_with_tls(fixture.smtp.clone())
        .is_err()
    );
    assert!(
        smtp.submit_with_tls(
            fixture.smtp.clone(),
            ACCOUNTS[0],
            &["outside@example.org".into()],
            b"Subject: no relay\r\n\r\nno"
        )
        .is_err()
    );
    let mut alice = fixture.incoming(&a)?;
    let mut bob = fixture.incoming(&b)?;
    assert!(alice.append_sent("Sent", b"Subject: literal\r\n\r\nsynthetic")?);
    let snapshot = alice.snapshot("Sent")?;
    assert_eq!(snapshot.messages.len(), 1);
    let uid = snapshot.messages[0].uid;
    let epoch = snapshot.epoch;
    assert!(bob.snapshot("Sent")?.messages.is_empty());
    alice.flag(uid, "flagged", true)?;
    drop(alice);
    let mut alice = fixture.incoming(&a)?;
    let snapshot = alice.snapshot("Sent")?;
    assert_eq!(snapshot.epoch, epoch);
    assert_eq!(snapshot.messages[0].uid, uid);
    assert!(snapshot.messages[0].starred);
    alice.move_message(uid, "Archive")?;
    assert!(alice.snapshot("Sent")?.messages.is_empty());
    let moved = alice.snapshot("Archive")?;
    assert_eq!(moved.messages.len(), 1);
    alice.remove_copied_source(moved.messages[0].uid)?;
    assert!(alice.snapshot("Archive")?.messages.is_empty());
    assert!(alice.append_sent("Archive", b"Subject: next\r\n\r\nnext")?);
    assert!(alice.snapshot("Archive")?.messages[0].uid > moved.messages[0].uid);
    Ok(())
}

#[test]
fn plaintext_auth_is_rejected_and_examine_cannot_mutate_flags() -> Result {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpStream,
        time::Duration,
    };
    let fixture = Fixture::new(false, false)?;
    let a = fixture.peer(0, TlsMode::Implicit)?;
    let socket = TcpStream::connect(("127.0.0.1", fixture.endpoints.imap_starttls))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut client = imap::Client::new(socket);
    client.read_greeting()?;
    assert!(client.login(ACCOUNTS[0], PASSWORD).is_err());

    let socket = TcpStream::connect(("127.0.0.1", fixture.endpoints.smtp_starttls))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut smtp = BufReader::new(socket);
    let mut line = String::new();
    smtp.read_line(&mut line)?;
    assert!(line.starts_with("220 "));
    smtp.get_mut()
        .write_all(b"AUTH PLAIN AGFsaWNlAHBhc3N3b3Jk\r\n")?;
    line.clear();
    smtp.read_line(&mut line)?;
    assert!(line.starts_with("503 "));
    smtp.get_mut()
        .write_all(b"MAIL FROM:<alice@moonmail.test>\r\n")?;
    line.clear();
    smtp.read_line(&mut line)?;
    assert!(line.starts_with("503 "));

    let socket = TcpStream::connect(("127.0.0.1", a.account.imap_port))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut client = imap::Client::new(fixture.imap.connect("localhost", socket)?);
    client.read_greeting()?;
    let mut session = client.login(ACCOUNTS[0], PASSWORD).map_err(|(e, _)| e)?;
    session
        .append("INBOX", b"Subject: read-only\r\n\r\nbody")
        .finish()?;
    session.examine("INBOX")?;
    assert!(session.uid_store("1", "+FLAGS (\\Seen)").is_err());
    session.select("INBOX")?;
    assert!(session.uid_fetch("1", "(BODY[] ENVELOPE)").is_err());
    let messages = session.uid_fetch("1", "(UID FLAGS)")?;
    assert_eq!(messages.len(), 1);
    assert!(
        !messages
            .iter()
            .next()
            .ok_or("missing fetched message")?
            .flags()
            .contains(&imap::types::Flag::Seen)
    );
    assert!(
        session.uid_fetch("999", "ENVELOPE").is_err(),
        "unsupported FETCH must not silently pass on empty results"
    );
    session.logout()?;
    Ok(())
}

#[test]
fn disabled_folder_is_listed_without_selecting_it() -> Result {
    let fixture = Fixture::new(false, false)?;
    let peer = fixture.peer(0, TlsMode::Implicit)?;
    let archive = Archive::open(peer.root.path())?;
    let inbox = archive.mailbox(&peer.account.email, "INBOX")?;
    archive.set_sync_enabled(inbox, false)?;
    fixture.sync(&peer)?;
    let folders = archive.mailboxes()?;
    assert!(
        folders
            .iter()
            .any(|f| f.id == inbox && f.epoch == 0 && !f.sync_enabled)
    );
    assert!(folders.iter().any(|f| f.id != inbox && f.epoch != 0));
    archive.set_sync_enabled(inbox, true)?;
    fixture.sync(&peer)?;
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|f| f.id == inbox && f.epoch != 0)
    );
    Ok(())
}

#[test]
fn remote_draft_round_trip_replace_pull_send_and_discard() -> Result {
    let fixture = Fixture::new(false, false)?;
    let a = fixture.peer(0, TlsMode::Implicit)?;
    let b = fixture.peer(0, TlsMode::Implicit)?;
    let mut draft = Draft {
        id: "editable".into(),
        account: a.account.email.clone(),
        body: "unfinished".into(),
        ..Default::default()
    };
    let store = Archive::open(a.root.path())?;
    store.save_draft(&draft)?;
    fixture.sync(&a)?;
    let mut imap = fixture.incoming(&a)?;
    let first = imap.snapshot("Drafts")?;
    assert_eq!(first.messages.len(), 1);
    assert!(first.messages[0].seen);
    let old = first.messages[0].uid;
    let mut observer = fixture.session(&a)?;
    observer.examine("Drafts")?;
    let flags = observer.uid_fetch(old.to_string(), "(UID FLAGS)")?;
    let flags = flags.iter().next().ok_or("missing remote draft")?.flags();
    assert!(flags.contains(&imap::types::Flag::Draft) && flags.contains(&imap::types::Flag::Seen));
    draft.subject = "revised".into();
    draft.bcc = ACCOUNTS[1].into();
    draft.attachments.push(DraftAttachment {
        id: "slot".into(),
        name: "sample.bin".into(),
        blob: FileBlobs::attachments(a.root.path()).put(b"test")?,
        bytes: 4,
    });
    store.save_draft(&draft)?;
    fixture.sync(&a)?;
    let next = imap.snapshot("Drafts")?;
    assert_eq!(next.messages.len(), 1);
    assert!(old < next.messages[0].uid);
    fixture.sync(&b)?;
    let other = Archive::open(b.root.path())?;
    let pulled = other.latest_draft()?.ok_or("expected remote draft")?;
    assert_eq!(pulled.subject, "revised");
    assert_eq!(pulled.body.trim(), "unfinished");
    assert_eq!(pulled.bcc, ACCOUNTS[1]);
    assert_eq!(
        FileBlobs::attachments(b.root.path()).get(&pulled.attachments[0].blob)?,
        b"test"
    );
    b.send(&fixture, &pulled, false)?;
    fixture.sync(&b)?;
    assert!(imap.snapshot("Drafts")?.messages.is_empty());
    fixture.sync(&a)?;
    assert_eq!(store.draft_count()?, 0);

    imap.append_draft(
        "Drafts",
        b"From: sender@example.org\r\nSubject: foreign\r\n\r\nexternal",
    )?;
    fixture.sync(&a)?;
    let foreign = store.latest_draft()?.ok_or("missing external draft")?;
    assert_eq!(foreign.subject, "foreign");
    store.discard_draft(&foreign.id)?;
    fixture.sync(&a)?;
    assert!(imap.snapshot("Drafts")?.messages.is_empty());
    assert_eq!(store.draft_count()?, 0);
    Ok(())
}

#[test]
fn incremental_drafts_keep_untouched_messages() -> Result {
    let fixture = Fixture::new(false, false)?;
    let peer = fixture.peer(0, TlsMode::Implicit)?;
    let mut session = fixture.incoming(&peer)?;
    session.append_draft("Drafts", b"Subject: earlier\r\n\r\none")?;
    fixture.sync(&peer)?;
    let archive = Archive::open(peer.root.path())?;
    let first = archive.latest_draft()?.ok_or("missing original draft")?;
    session.append_draft("Drafts", b"Subject: later\r\n\r\ntwo")?;
    fixture.sync(&peer)?;
    assert_eq!(archive.draft_count()?, 2);
    assert_eq!(
        archive
            .draft(&first.id)?
            .ok_or("original draft retired")?
            .subject,
        "earlier"
    );
    Ok(())
}

#[test]
fn permanent_delete_is_bound_to_folder_epoch_and_confirmed_uids() -> Result {
    use moonmail_store::DeletionTarget;
    let fixture = Fixture::new(false, false)?;
    let a = fixture.peer(0, TlsMode::Implicit)?;
    let mut imap = fixture.incoming(&a)?;
    for role in [MailboxRole::Trash, MailboxRole::Junk] {
        let folder = if role == MailboxRole::Trash {
            "Trash"
        } else {
            "Junk"
        };
        imap.append_sent(folder, b"Subject: first\r\n\r\nfirst")?;
        imap.append_sent(folder, b"Subject: second\r\n\r\nsecond")?;
        fixture.sync(&a)?;
        let mut store = Archive::open(a.root.path())?;
        let mailbox = store.role_mailbox(&a.account.email, role)?;
        let snapshot = imap.snapshot(folder)?;
        let first = snapshot.messages[0].uid;
        let upper = snapshot.messages[1].uid;
        let mut observer = fixture.session(&a)?;
        observer.select(folder)?;
        observer.uid_store(upper.to_string(), "+FLAGS.SILENT (\\Deleted)")?;
        assert_eq!(
            imap.expunge_uid(folder, snapshot.epoch.wrapping_add(1), first)?,
            moonmail_network::ExpungeOutcome::Stale
        );
        assert_eq!(imap.snapshot(folder)?.messages.len(), 2);
        let flags = observer.uid_fetch(first.to_string(), "(UID FLAGS)")?;
        assert!(
            !flags
                .iter()
                .next()
                .ok_or("stale preflight removed UID")?
                .flags()
                .contains(&imap::types::Flag::Deleted)
        );
        let page = store.page(
            &moonmail_core::MailQuery {
                mailbox: Some(mailbox),
                ..Default::default()
            },
            10,
        )?;
        let first_id = page
            .rows
            .iter()
            .find(|row| row.subject == "first")
            .ok_or("first occurrence")?
            .id;
        store.queue_expunge(
            &a.account.email,
            mailbox,
            folder,
            snapshot.epoch,
            role,
            DeletionTarget::Message(first_id, first),
        )?;
        fixture.sync(&a)?;
        assert_eq!(imap.snapshot(folder)?.messages.len(), 1);
        store.queue_expunge(
            &a.account.email,
            mailbox,
            folder,
            snapshot.epoch,
            role,
            DeletionTarget::Through(upper),
        )?;
        imap.append_sent(folder, b"Subject: later\r\n\r\nkeep")?;
        fixture.sync(&a)?;
        let remaining = imap.snapshot(folder)?;
        assert_eq!(remaining.messages.len(), 1);
        assert!(remaining.messages[0].uid > upper);
    }
    Ok(())
}

#[test]
fn scan_recovers_append_without_acknowledgement_by_message_id() -> Result {
    let fixture = Fixture::new(false, false)?;
    let peer = fixture.peer(0, TlsMode::Implicit)?;
    let mut archive = Archive::open(peer.root.path())?;
    let draft = Draft {
        id: "interrupted".into(),
        subject: "unfinished".into(),
        account: peer.account.email.clone(),
        ..Default::default()
    };
    archive.save_draft(&draft)?;
    let (_, revision) = archive
        .dirty_draft(&draft.account, 0)?
        .ok_or("missing revision")?;
    let message_id = "synthetic-attempt@moonmail.local";
    assert!(archive.claim_draft_upload(&draft, revision, "Drafts", message_id)?);
    let mut incoming = fixture.incoming(&peer)?;
    incoming.append_draft(
        "Drafts",
        format!("Message-ID: <{message_id}>\r\nSubject: unfinished\r\n\r\n").as_bytes(),
    )?;
    // The socket worker stopped before writing the APPEND receipt.
    drop(archive);
    fixture.sync(&peer)?;
    let mut archive = Archive::open(peer.root.path())?;
    assert_eq!(archive.unresolved_remote_count()?, 0);
    assert_eq!(archive.draft_count()?, 1);
    assert!(archive.dirty_draft(&draft.account, 0)?.is_none());
    assert_eq!(incoming.snapshot("Drafts")?.messages.len(), 1);
    archive.discard_draft(&draft.id)?;
    fixture.sync(&peer)?;
    assert!(incoming.snapshot("Drafts")?.messages.is_empty());
    Ok(())
}

#[test]
fn condstore_delta_recovers_after_cancellation_and_reconciles_expunge() -> Result {
    let fixture = Fixture::new(false, false)?;
    let peer = fixture.peer(0, TlsMode::Implicit)?;
    let mut remote = fixture.incoming(&peer)?;
    for uid in 1..=3 {
        remote.append_sent("INBOX", format!("Subject: {uid}\r\n\r\nFixture").as_bytes())?;
    }
    let mut archive = Archive::open(peer.root.path())?;
    let mailbox = archive.mailbox(&peer.account.email, "INBOX")?;
    let stop = std::cell::Cell::new(false);
    let result = synchronize_cancellable(
        &mut remote,
        &peer.account,
        &mut archive,
        &FileBlobs::new(peer.root.path()),
        |s| {
            if s.starts_with("Downloaded") {
                stop.set(true)
            }
        },
        || stop.get(),
    );
    assert!(matches!(
        result,
        Err(moonmail_network::SyncError::Cancelled)
    ));
    assert_eq!(archive.total()?, 1);
    assert_eq!(archive.sync_checkpoint(mailbox)?, None);
    fixture.sync(&peer)?;
    let before = archive
        .sync_checkpoint(mailbox)?
        .ok_or("no initial checkpoint")?;
    fixture.sync(&peer)?;
    assert_eq!(archive.sync_checkpoint(mailbox)?, Some(before));
    remote.select_uid("INBOX", 2)?;
    remote.flag(2, "flagged", true)?;
    remote.remove_copied_source(1)?;
    remote.append_sent("INBOX", b"Subject: 4\r\n\r\nFixture")?;
    fixture.sync(&peer)?;
    let after = archive
        .sync_checkpoint(mailbox)?
        .ok_or("no delta checkpoint")?;
    assert_eq!(after.upper, 4);
    assert!(after.highest_modseq > before.highest_modseq);
    let rows = archive.page(&Default::default(), 10)?.rows;
    assert_eq!(rows.len(), 3);
    assert!(rows.iter().any(|r| r.subject == "2" && r.starred));
    assert!(rows.iter().any(|r| r.subject == "4"));
    assert!(rows.iter().all(|r| r.subject != "1"));
    assert_eq!(archive.total()?, 4);
    Ok(())
}

#[test]
fn live_workers_push_flush_and_pause_independently() -> Result {
    use {
        moonmail_network::{LiveAccount, LiveStatus},
        std::{
            sync::atomic::{AtomicUsize, Ordering},
            time::{Duration, Instant},
        },
    };

    fn until(mut observed: impl FnMut() -> bool) -> Result {
        let end = Instant::now() + Duration::from_secs(5);
        while !observed() {
            if Instant::now() >= end {
                return Err("live worker missed fixture deadline".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    let fixture = Fixture::new(false, false)?;
    let server = fixture._server.as_ref().ok_or("expected native fixture")?;
    let healthy = fixture.peer(0, TlsMode::Implicit)?;
    let failing = fixture.peer(1, TlsMode::Implicit)?;
    let root = healthy.root.path();
    let mut archive = Archive::open(root)?;
    let tls = fixture.imap.clone();
    let (release_login, gate) = std::sync::mpsc::channel();
    let bad = LiveAccount::start_with(root.into(), failing.account, 15, move |a| {
        gate.recv_timeout(Duration::from_secs(10))
            .map_err(|_| moonmail_network::Error::Invalid("Fixture login gate expired"))?;
        let mut credentials = secret();
        credentials.value = "wrong synthetic password".into();
        RustImap::connect_with_tls(a, &credentials, &tls)
    })?;
    let tls = fixture.imap.clone();
    let connections = Arc::new(AtomicUsize::new(0));
    let count = connections.clone();
    let live = LiveAccount::start_with(root.into(), healthy.account.clone(), 15, move |a| {
        count.fetch_add(1, Ordering::Relaxed);
        RustImap::connect_with_tls(a, &secret(), &tls)
    })?;
    // The healthy account reaches IDLE while its peer is still connecting.
    until(|| server.idle_counts() == (1, 0))?;
    release_login.send(())?;
    until(|| {
        bad.events()
            .any(|event| matches!(event, LiveStatus::Error { retry: None, .. }))
    })?;
    let raw = b"Subject: push\r\nMessage-ID: <push@moonmail.test>\r\nFrom: sender@moonmail.test\r\nTo: bob@moonmail.test\r\n\r\nfixture body\r\n";
    let mut probe = fixture.incoming(&healthy)?;
    probe.append_sent("INBOX", raw)?;
    until(|| server.idle_counts() == (2, 1))?;
    let id = healthy.observe(raw, MailboxRole::Inbox)?;
    archive.set_flag(id, "flagged", true)?;
    live.flush();
    until(|| server.idle_counts() == (3, 2))?;
    assert!(probe.snapshot("INBOX")?.messages.iter().any(|m| m.starred));
    let uid = probe
        .snapshot("INBOX")?
        .messages
        .first()
        .ok_or("missing remote message")?
        .uid;
    probe.flag(uid, "flagged", false)?;
    until(|| server.idle_counts() == (4, 3))?;
    assert!(!archive.detail(id)?.summary.starred);
    archive.queue_moves(
        &[id],
        None,
        moonmail_store::MoveTarget::Role(MailboxRole::Archive),
    )?;
    live.flush();
    until(|| server.idle_counts() == (5, 4))?;
    assert!(probe.snapshot("INBOX")?.messages.is_empty());
    healthy.observe(raw, MailboxRole::Archive)?;
    assert_eq!(connections.load(Ordering::Relaxed), 1);
    live.pause();
    until(|| server.idle_counts() == (5, 5))?;
    until(|| live.events().any(|s| matches!(s, LiveStatus::Paused)))?;
    live.flush();
    std::thread::sleep(Duration::from_millis(350));
    assert_eq!(server.idle_counts(), (5, 5));
    live.sync();
    until(|| server.idle_counts() == (6, 5))?;
    drop(live);
    until(|| server.idle_counts() == (6, 6))?;
    Ok(())
}
