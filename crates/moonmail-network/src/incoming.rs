use {
    crate::{Account, Error, LoginSecret, Result, oauth_bearer},
    std::{
        net::{TcpStream, ToSocketAddrs},
        time::Duration,
    },
};

pub struct RemoteFolder {
    pub name: String,
    pub delimiter: Option<String>,
    pub selectable: bool,
}
pub struct RemoteMessage {
    pub uid: u32,
    pub seen: bool,
    pub starred: bool,
    pub size: u32,
}
pub struct RemoteSnapshot {
    pub epoch: u32,
    pub messages: Vec<RemoteMessage>,
}

/// A connection-owning, blocking adapter. A snapshot is authoritative only after
/// successful SELECT and complete UID/flag enumeration. Fetch must not mark read.
pub trait IncomingMail {
    type Error: std::error::Error + Send + Sync + 'static;
    fn folders(&mut self) -> std::result::Result<Vec<RemoteFolder>, Self::Error>;
    fn snapshot(&mut self, folder: &str) -> std::result::Result<RemoteSnapshot, Self::Error>;
    fn fetch(&mut self, uid: u32) -> std::result::Result<Option<Vec<u8>>, Self::Error>;
    fn flag(&mut self, uid: u32, kind: &str, value: bool) -> std::result::Result<(), Self::Error>;
}

pub struct RustImap {
    session: imap::Session<native_tls::TlsStream<TcpStream>>,
}

struct Bearer(String);
impl imap::Authenticator for Bearer {
    type Response = Vec<u8>;
    fn process(&self, challenge: &[u8]) -> Vec<u8> {
        if challenge.is_empty() {
            self.0.as_bytes().to_vec()
        } else {
            vec![1]
        }
    }
}

impl RustImap {
    pub fn connect(account: &Account, secret: &LoginSecret) -> Result<Self> {
        let timeout = Duration::from_secs(30);
        let mut socket = None;
        for address in (account.imap_host.as_str(), account.imap_port).to_socket_addrs()? {
            if let Ok(stream) = TcpStream::connect_timeout(&address, timeout) {
                socket = Some(stream);
                break;
            }
        }
        let socket = socket.ok_or(Error::Invalid("Could not connect to IMAP"))?;
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        let tls = native_tls::TlsConnector::new()?
            .connect(&account.imap_host, socket)
            .map_err(|_| Error::Invalid("IMAP TLS handshake failed"))?;
        let mut client = imap::Client::new(tls);
        client.read_greeting()?;
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
                        account.email, secret.value
                    ),
                )
            } else {
                return Err(Error::Invalid(
                    "Server does not advertise OAuth IMAP authentication",
                ));
            };
            client
                .authenticate(mechanism, &Bearer(payload))
                .map_err(|(error, _)| error)?
        } else {
            client
                .login(&account.email, &secret.value)
                .map_err(|(error, _)| error)?
        };
        Ok(Self { session })
    }
}

impl IncomingMail for RustImap {
    type Error = Error;
    fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        Ok(self
            .session
            .list(None, Some("*"))?
            .iter()
            .map(|folder| RemoteFolder {
                name: folder.name().to_owned(),
                delimiter: folder.delimiter().map(str::to_owned),
                selectable: !folder
                    .attributes()
                    .contains(&imap_proto::NameAttribute::NoSelect),
            })
            .collect())
    }
    fn snapshot(&mut self, folder: &str) -> Result<RemoteSnapshot> {
        let selected = self.session.select(folder)?;
        let epoch = selected
            .uid_validity
            .ok_or(Error::Invalid("Server omitted UIDVALIDITY"))?;
        let mut messages = Vec::new();
        if selected.exists > 0 {
            for fetch in self
                .session
                .uid_fetch("1:*", "(UID FLAGS RFC822.SIZE)")?
                .iter()
            {
                messages.push(RemoteMessage {
                    uid: fetch.uid.ok_or(Error::Invalid("Server omitted UID"))?,
                    seen: fetch.flags().contains(&imap::types::Flag::Seen),
                    starred: fetch.flags().contains(&imap::types::Flag::Flagged),
                    size: fetch
                        .size
                        .ok_or(Error::Invalid("Server omitted message size"))?,
                });
            }
        }
        Ok(RemoteSnapshot { epoch, messages })
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
}
