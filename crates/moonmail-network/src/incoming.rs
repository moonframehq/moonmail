use {
    crate::{
        Account, Error, ExposeSecret, LoginSecret, Provider, Result, SecretString, TlsMode,
        oauth_bearer,
    },
    moonmail_core::{FolderChange, MailboxRole},
    std::{
        net::{TcpStream, ToSocketAddrs},
        time::Duration,
    },
};

pub struct RemoteFolder {
    pub name: String,
    pub delimiter: Option<String>,
    pub selectable: bool,
    pub role: Option<MailboxRole>,
}
pub struct RemoteMessage {
    pub uid: u32,
    pub seen: bool,
    pub starred: bool,
    pub size: u32,
    /// Server INTERNALDATE as UTC Unix seconds, not the download time or Date header.
    pub received_at: i64,
    pub keywords: Vec<String>,
}
pub struct RemoteSnapshot {
    pub epoch: u32,
    pub messages: Vec<RemoteMessage>,
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError<I, S> {
    #[error("mailbox inventory failed")]
    Incoming(#[source] I),
    #[error("mailbox inventory consumer stopped")]
    Sink(#[source] S),
}

pub struct RemoteUid {
    pub epoch: u32,
    pub exists: bool,
}

pub enum MoveOutcome {
    Moved,
    Unsupported,
}

#[derive(Debug, PartialEq, Eq)]
pub enum KeywordOutcome {
    Applied,
    Unsupported,
    Stale,
}

pub enum FolderOutcome {
    Applied,
    /// No namespace mutation was dispatched.
    Unsupported,
}

pub enum CopyOutcome {
    /// No mutating command was sent; targeted removal is unavailable as well.
    Unsupported,
    /// COPY succeeded, but the server supplied no usable one-message mapping.
    Unverified,
    Copied {
        epoch: u32,
        uid: u32,
    },
}

/// A connection-owning, blocking adapter. A snapshot is authoritative only after
/// successful SELECT and complete UID/flag enumeration. Fetch must not mark read.
pub trait IncomingMail {
    type Error: std::error::Error + Send + Sync + 'static;
    /// Append exact accepted MIME to Sent. False means unsupported and no
    /// command was dispatched; an error may follow a committed append.
    fn append_sent(
        &mut self,
        _mailbox: &str,
        _raw: &[u8],
    ) -> std::result::Result<bool, Self::Error> {
        Ok(false)
    }
    /// The complete LIST namespace, including unselectable folders. Return an
    /// error, never a partial list, if enumeration or its final response fails.
    /// This is not a subscription-filtered list: absence retires remote state.
    fn folders(&mut self) -> std::result::Result<Vec<RemoteFolder>, Self::Error>;
    /// Read-only epoch precondition for a selectable folder. Native adapters
    /// should use EXAMINE rather than enumerating its messages.
    fn folder_epoch(&mut self, folder: &str) -> std::result::Result<u32, Self::Error> {
        Ok(self.snapshot(folder)?.epoch)
    }
    /// Exactly one CREATE/RENAME/DELETE command. Any error may follow a committed
    /// namespace mutation. Never retry automatically or expunge on closing.
    fn edit_folder(
        &mut self,
        _change: &FolderChange,
    ) -> std::result::Result<FolderOutcome, Self::Error> {
        Ok(FolderOutcome::Unsupported)
    }
    fn snapshot(&mut self, folder: &str) -> std::result::Result<RemoteSnapshot, Self::Error>;
    /// Enumerate metadata in batches of at most 1000, within one SELECT epoch
    /// and a fixed inclusive UID upper bound. Call the sink even when empty.
    /// Only Ok means the whole scan completed. Errors/cancellation must never
    /// make a partial inventory authoritative. Concurrent arrivals beyond the
    /// upper bound are deferred; expunges may disappear on the next scan.
    /// The compatibility default materializes snapshot; native adapters override.
    fn scan<S>(
        &mut self,
        folder: &str,
        mut sink: impl FnMut(u32, u32, &[RemoteMessage]) -> std::result::Result<(), S>,
    ) -> std::result::Result<(), ScanError<Self::Error, S>> {
        let snapshot = self.snapshot(folder).map_err(ScanError::Incoming)?;
        // Legacy snapshots cover the entire namespace, including an empty
        // mailbox after its highest UID was expunged. Max(observed UID) is not
        // a valid deletion boundary in that case.
        let upper = u32::MAX;
        sink(snapshot.epoch, upper, &[]).map_err(ScanError::Sink)?;
        for batch in snapshot.messages.chunks(1000) {
            sink(snapshot.epoch, upper, batch).map_err(ScanError::Sink)?;
        }
        Ok(())
    }
    fn fetch(&mut self, uid: u32) -> std::result::Result<Option<Vec<u8>>, Self::Error>;
    fn flag(&mut self, uid: u32, kind: &str, value: bool) -> std::result::Result<(), Self::Error>;
    /// Select and validate UIDVALIDITY/UID and permanent keyword support before
    /// idempotent +/-FLAGS.SILENT. Errors remain retryable; never replace FLAGS.
    fn keyword(
        &mut self,
        _folder: &str,
        _epoch: u32,
        _uid: u32,
        _keyword: &str,
        _value: bool,
    ) -> std::result::Result<KeywordOutcome, Self::Error> {
        Ok(KeywordOutcome::Unsupported)
    }
    /// Select the source and validate one UID without mutating it. The selected
    /// mailbox remains current for flags/MOVE. Native adapters should bound this
    /// probe to one UID instead of enumerating the whole folder for every move.
    fn select_uid(
        &mut self,
        folder: &str,
        uid: u32,
    ) -> std::result::Result<RemoteUid, Self::Error> {
        let snapshot = self.snapshot(folder)?;
        Ok(RemoteUid {
            epoch: snapshot.epoch,
            exists: snapshot.messages.iter().any(|message| message.uid == uid),
        })
    }
    /// Move one UID from the selected mailbox. Unsupported means no command was
    /// sent. Any error may follow a successful remote mutation and is uncertain.
    /// This method never performs a copy/delete fallback itself.
    fn move_message(
        &mut self,
        _uid: u32,
        _destination: &str,
    ) -> std::result::Result<MoveOutcome, Self::Error> {
        Ok(MoveOutcome::Unsupported)
    }

    /// Copy one selected UID only when targeted source removal is supported.
    /// Copied identifies the destination UID/UIDVALIDITY from the acknowledged
    /// COPY response. Errors may follow a committed copy. Never remove source.
    fn copy_for_move(
        &mut self,
        _uid: u32,
        _destination: &str,
    ) -> std::result::Result<CopyOutcome, Self::Error> {
        Ok(CopyOutcome::Unsupported)
    }

    /// Copy without source removal. Unverified means COPY was acknowledged but
    /// provided no usable UID mapping; that is a completed standalone copy.
    /// Any error may follow a committed copy and must not trigger automatic retry.
    fn copy_message(
        &mut self,
        _uid: u32,
        _destination: &str,
    ) -> std::result::Result<CopyOutcome, Self::Error> {
        Ok(CopyOutcome::Unsupported)
    }

    /// SHA-256 of exact selected-message bytes, or None for missing/oversized
    /// messages. Native implementations must bound the body fetch to 64 MiB.
    fn message_digest(&mut self, uid: u32) -> std::result::Result<Option<String>, Self::Error> {
        use sha2::{Digest, Sha256};
        Ok(self
            .fetch(uid)?
            .filter(|raw| raw.len() <= 64 * 1024 * 1024)
            .map(|raw| {
                Sha256::digest(raw)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect()
            }))
    }

    /// Mark only this UID deleted, then UID EXPUNGE only this UID. Never issue
    /// ordinary EXPUNGE or CLOSE. Caller has verified the destination bytes and
    /// reselected/revalidated the source epoch. Errors are uncertain.
    fn remove_copied_source(&mut self, _uid: u32) -> std::result::Result<MoveOutcome, Self::Error> {
        Ok(MoveOutcome::Unsupported)
    }
}

pub struct RustImap {
    session: imap::Session<native_tls::TlsStream<TcpStream>>,
    provider: Provider,
}

struct Bearer {
    payload: SecretString,
    xoauth2: bool,
}

impl imap::Authenticator for Bearer {
    type Response = Vec<u8>;
    fn process(&self, challenge: &[u8]) -> Vec<u8> {
        if challenge.is_empty() {
            self.payload.expose_secret().as_bytes().to_vec()
        } else if self.xoauth2 {
            Vec::new()
        } else {
            vec![1]
        }
    }
}

impl RustImap {
    pub fn connect(account: &Account, secret: &LoginSecret) -> Result<Self> {
        Self::connect_with_tls(account, secret, &native_tls::TlsConnector::new()?)
    }

    /// Connect with caller-supplied trust roots (for private servers and fixtures).
    pub fn connect_with_tls(
        account: &Account,
        secret: &LoginSecret,
        connector: &native_tls::TlsConnector,
    ) -> Result<Self> {
        account.validate()?;
        let timeout = Duration::from_secs(30);
        let mut socket = None;
        let mut last_error = None;
        for address in (account.imap_host.as_str(), account.imap_port).to_socket_addrs()? {
            match TcpStream::connect_timeout(&address, timeout) {
                Ok(stream) => {
                    socket = Some(stream);
                    break;
                },
                Err(error) => last_error = Some(error),
            }
        }
        let socket = socket.ok_or_else(|| {
            last_error
                .map(Error::Io)
                .unwrap_or(Error::Invalid("No IMAP socket addresses resolved"))
        })?;
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        let greeted = account.imap_tls == TlsMode::Starttls;
        let socket = if greeted {
            let mut client = imap::Client::new(socket);
            client.read_greeting()?;
            crate::imap_tls::upgrade(client)?
        } else {
            socket
        };
        let tls = connector
            .connect(&account.imap_host, socket)
            .map_err(|_| Error::Invalid("IMAP TLS handshake failed"))?;
        let mut client = imap::Client::new(tls);
        if !greeted {
            client.read_greeting()?;
        }
        let session = if secret.bearer {
            let capabilities = client.capabilities()?;
            let (mechanism, payload) = if capabilities.has_str("AUTH=OAUTHBEARER") {
                (
                    "OAUTHBEARER",
                    oauth_bearer(&account.imap_host, account.imap_port, &secret.value),
                )
            } else if capabilities.has_str("AUTH=XOAUTH2") {
                (
                    "XOAUTH2",
                    format!(
                        "user={}\x01auth=Bearer {}\x01\x01",
                        account.imap_login(),
                        secret.value.expose_secret()
                    )
                    .into(),
                )
            } else {
                return Err(Error::Invalid(
                    "Server does not advertise OAuth IMAP authentication",
                ));
            };
            client
                .authenticate(
                    mechanism,
                    &Bearer {
                        payload,
                        xoauth2: mechanism == "XOAUTH2",
                    },
                )
                .map_err(|(error, _)| error)?
        } else {
            client
                .login(account.imap_login(), secret.value.expose_secret())
                .map_err(|(error, _)| error)?
        };
        Ok(Self {
            session,
            provider: account.provider,
        })
    }
}

impl IncomingMail for RustImap {
    type Error = Error;
    fn append_sent(&mut self, mailbox: &str, raw: &[u8]) -> Result<bool> {
        self.session
            .append(mailbox, raw)
            .flag(imap::types::Flag::Seen)
            .finish()?;
        Ok(true)
    }
    fn keyword(
        &mut self,
        folder: &str,
        epoch: u32,
        uid: u32,
        keyword: &str,
        value: bool,
    ) -> Result<KeywordOutcome> {
        crate::imap_keywords::store(&mut self.session, folder, epoch, uid, keyword, value)
    }
    fn folder_epoch(&mut self, folder: &str) -> Result<u32> {
        self.session
            .examine(folder)?
            .uid_validity
            .ok_or(Error::Invalid("Server omitted folder UIDVALIDITY"))
    }
    fn edit_folder(&mut self, change: &FolderChange) -> Result<FolderOutcome> {
        crate::imap_folders::edit(&mut self.session, change)?;
        Ok(FolderOutcome::Applied)
    }
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        if self.session.capabilities()?.has_str("SPECIAL-USE") {
            let response = self
                .session
                .run_command_and_read_response("LIST \"\" \"*\" RETURN (SPECIAL-USE)")?;
            let mut folders = crate::folders::parse_list(&response)?;
            crate::folders::provider_roles(self.provider, &mut folders);
            return Ok(folders);
        }
        let mut folders: Vec<_> = self
            .session
            .list(None, Some("*"))?
            .iter()
            .map(|folder| RemoteFolder {
                name: folder.name().to_owned(),
                delimiter: folder.delimiter().map(str::to_owned),
                selectable: !folder
                    .attributes()
                    .contains(&imap_proto::NameAttribute::NoSelect),
                role: crate::folders::role(folder.name(), folder.attributes()),
            })
            .collect();
        crate::folders::provider_roles(self.provider, &mut folders);
        Ok(folders)
    }
    fn snapshot(&mut self, folder: &str) -> Result<RemoteSnapshot> {
        let mut epoch = 0;
        let mut messages = Vec::new();
        self.scan(folder, |value, _, batch| {
            epoch = value;
            messages.extend(batch.iter().map(|m| RemoteMessage {
                keywords: m.keywords.clone(),
                uid: m.uid,
                seen: m.seen,
                starred: m.starred,
                size: m.size,
                received_at: m.received_at,
            }));
            Ok::<_, std::convert::Infallible>(())
        })
        .map_err(|error| match error {
            ScanError::Incoming(error) => error,
            ScanError::Sink(never) => match never {},
        })?;
        Ok(RemoteSnapshot { epoch, messages })
    }

    fn scan<S>(
        &mut self,
        folder: &str,
        sink: impl FnMut(u32, u32, &[RemoteMessage]) -> std::result::Result<(), S>,
    ) -> std::result::Result<(), ScanError<Self::Error, S>> {
        crate::imap_inventory::scan(&mut self.session, folder, sink)
    }
    fn fetch(&mut self, uid: u32) -> Result<Option<Vec<u8>>> {
        Ok(self
            .session
            .uid_fetch(uid.to_string(), "(UID BODY.PEEK[])")?
            .iter()
            .find(|fetch| fetch.uid == Some(uid))
            .and_then(|fetch| fetch.body().map(<[u8]>::to_vec)))
    }
    fn flag(&mut self, uid: u32, kind: &str, value: bool) -> Result<()> {
        let flag = match kind {
            "seen" => "\\Seen",
            "flagged" => "\\Flagged",
            _ => return Err(Error::Invalid("Unknown flag")),
        };
        self.session.uid_store(
            uid.to_string(),
            format!("{}FLAGS.SILENT ({flag})", if value { "+" } else { "-" }),
        )?;
        Ok(())
    }

    fn move_message(&mut self, uid: u32, destination: &str) -> Result<MoveOutcome> {
        if !self.session.capabilities()?.has_str("MOVE") {
            return Ok(MoveOutcome::Unsupported);
        }
        self.session.uid_mv(uid.to_string(), destination)?;
        Ok(MoveOutcome::Moved)
    }

    fn copy_for_move(&mut self, uid: u32, destination: &str) -> Result<CopyOutcome> {
        crate::imap_copy::copy(&mut self.session, uid, destination, true)
    }

    fn copy_message(&mut self, uid: u32, destination: &str) -> Result<CopyOutcome> {
        crate::imap_copy::copy(&mut self.session, uid, destination, false)
    }

    fn message_digest(&mut self, uid: u32) -> Result<Option<String>> {
        crate::imap_copy::digest(&mut self.session, uid)
    }

    fn remove_copied_source(&mut self, uid: u32) -> Result<MoveOutcome> {
        crate::imap_copy::remove(&mut self.session, uid)
    }

    fn select_uid(&mut self, folder: &str, uid: u32) -> Result<RemoteUid> {
        let selected = self.session.select(folder)?;
        let epoch = selected
            .uid_validity
            .ok_or(Error::Invalid("Server omitted UIDVALIDITY"))?;
        let exists = self
            .session
            .uid_fetch(uid.to_string(), "(UID)")?
            .iter()
            .any(|fetch| fetch.uid == Some(uid));
        Ok(RemoteUid { epoch, exists })
    }
}

#[cfg(test)]
mod auth_tests {
    use {super::*, imap::Authenticator};
    #[test]
    fn xoauth2_failure_challenge_uses_empty_response() {
        let xoauth = Bearer {
            payload: "synthetic".into(),
            xoauth2: true,
        };
        assert_eq!(xoauth.process(b""), b"synthetic");
        assert!(xoauth.process(b"{\"status\":\"401\"}").is_empty());
        let bearer = Bearer {
            payload: "synthetic".into(),
            xoauth2: false,
        };
        assert_eq!(bearer.process(b"failure"), vec![1]);
    }
}
