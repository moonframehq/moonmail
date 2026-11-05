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

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Fixture {
    _server: Option<Server>,
    endpoints: Endpoints,
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
            imap,
            smtp: Arc::new(config).into(),
        })
    }

    fn peer(&self, index: usize, mode: TlsMode) -> Result<Peer> {
        let mut account = Account::preset(Provider::Other, ACCOUNTS[index])?;
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
            root: tempfile::tempdir()?,
        })
    }

    fn incoming(&self, peer: &Peer) -> Result<RustImap> {
        Ok(RustImap::connect_with_tls(
            &peer.account,
            &secret(),
            &self.imap,
        )?)
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
                    secret: secret(),
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
        assert_eq!(ids.len(), 1, "expected exactly one observed remote copy");
        let id = moonmail_core::MessageId(u64::try_from(ids[0])?);
        let actual = FileBlobs::new(self.root.path()).get(&archive.message_blob(id)?)?;
        // mail-send terminates DATA with an additional CRLF before the dot.
        assert!(
            actual == raw || actual == [raw, b"\r\n"].concat(),
            "received MIME bytes changed beyond SMTP framing"
        );
        Ok(id)
    }
}

fn round_trip(fixture: &Fixture, mode: TlsMode, server_sent: bool) -> Result {
    let a = fixture.peer(0, mode)?;
    let b = fixture.peer(1, mode)?;
    for peer in [&a, &b] {
        SmtpSubmission {
            account: peer.account.clone(),
            secret: secret(),
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
    let (a_outbox, raw) = a.send(fixture, &draft, server_sent)?;
    fixture.sync(&b)?;
    let received = b.observe(&raw, MailboxRole::Inbox)?;
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
    a.observe(&response, MailboxRole::Inbox)?;
    b.observe(&raw, MailboxRole::Inbox)?;
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
