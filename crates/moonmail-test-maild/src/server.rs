use {
    crate::{Error, Result, store::Store, wire::Wire},
    rcgen::{
        BasicConstraints, CertificateParams, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
        KeyUsagePurpose,
    },
    rustls::{ServerConfig, pki_types::PrivatePkcs8KeyDer},
    serde::{Deserialize, Serialize},
    std::{
        collections::BTreeMap,
        net::{IpAddr, Ipv4Addr, Shutdown, TcpListener, TcpStream},
        sync::{
            Arc, Mutex, MutexGuard,
            atomic::{AtomicBool, AtomicU64, Ordering},
        },
        thread::{self, JoinHandle},
        time::Duration,
    },
};

pub struct Config {
    pub bind: IpAddr,
    pub ports: [u16; 4],
    pub server_sent: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            bind: Ipv4Addr::LOCALHOST.into(),
            ports: [0; 4],
            server_sent: false,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Endpoints {
    pub imap_tls: u16,
    pub smtp_tls: u16,
    pub imap_starttls: u16,
    pub smtp_starttls: u16,
}

pub(crate) struct Shared {
    store: Mutex<Store>,
    pub tls: Arc<ServerConfig>,
    stop: AtomicBool,
    sockets: Mutex<BTreeMap<u64, TcpStream>>,
    next: AtomicU64,
}
impl Shared {
    pub fn store(&self) -> Result<MutexGuard<'_, Store>> {
        self.store
            .lock()
            .map_err(|_| Error::Invalid("Fixture state lock poisoned"))
    }
}

pub struct Server {
    pub endpoints: Endpoints,
    pub ca_der: Vec<u8>,
    pub ca_pem: String,
    shared: Arc<Shared>,
    listeners: Vec<JoinHandle<()>>,
}

impl Server {
    pub fn start(config: Config) -> Result<Self> {
        let (tls, ca_der, ca_pem) = certificates()?;
        let sockets: Vec<_> = config
            .ports
            .into_iter()
            .map(|port| TcpListener::bind((config.bind, port)))
            .collect::<std::io::Result<_>>()?;
        let ports: Vec<_> = sockets
            .iter()
            .map(|s| s.local_addr().map(|a| a.port()))
            .collect::<std::io::Result<_>>()?;
        for socket in &sockets {
            socket.set_nonblocking(true)?;
        }
        let mut server = Self {
            endpoints: Endpoints {
                imap_tls: ports[0],
                smtp_tls: ports[1],
                imap_starttls: ports[2],
                smtp_starttls: ports[3],
            },
            ca_der,
            ca_pem,
            shared: Arc::new(Shared {
                store: Mutex::new(Store::new(config.server_sent)),
                tls: Arc::new(tls),
                stop: AtomicBool::new(false),
                sockets: Mutex::new(BTreeMap::new()),
                next: AtomicU64::new(1),
            }),
            listeners: Vec::new(),
        };
        for (index, socket) in sockets.into_iter().enumerate() {
            let shared = Arc::clone(&server.shared);
            server.listeners.push(
                thread::Builder::new()
                    .name(format!("test-mail-listener-{index}"))
                    .spawn(move || accept(socket, shared, index % 2 == 0, index < 2))?,
            );
        }
        Ok(server)
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Ok(sockets) = self.shared.sockets.lock() {
            for socket in sockets.values() {
                let _ = socket.shutdown(Shutdown::Both);
            }
        }
        for listener in self.listeners.drain(..) {
            let _ = listener.join();
        }
    }
}

fn accept(listener: TcpListener, shared: Arc<Shared>, imap: bool, implicit: bool) {
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    while !shared.stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((socket, _)) => {
                if socket.set_nonblocking(false).is_err() {
                    continue;
                }
                let id = shared.next.fetch_add(1, Ordering::Relaxed);
                let Ok(copy) = socket.try_clone() else {
                    continue;
                };
                if socket
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .is_err()
                    || socket
                        .set_write_timeout(Some(Duration::from_secs(10)))
                        .is_err()
                {
                    continue;
                }
                let Ok(mut registered) = shared.sockets.lock() else {
                    break;
                };
                if registered.len() >= 32 || shared.stop.load(Ordering::Acquire) {
                    continue;
                }
                registered.insert(id, copy);
                drop(registered);
                let state = Arc::clone(&shared);
                match thread::Builder::new()
                    .name("test-mail-session".into())
                    .spawn(move || {
                        let mut wire = Wire::new(socket);
                        let ready = !implicit || wire.upgrade(Arc::clone(&state.tls)).is_ok();
                        if ready {
                            let result = if imap {
                                crate::imap::serve(&mut wire, &state)
                            } else {
                                crate::smtp::serve(&mut wire, &state)
                            };
                            if let Err(Error::Invalid(reason)) = result {
                                eprintln!("Fixture session ended: {reason}");
                            }
                        }
                        if let Ok(mut sockets) = state.sockets.lock() {
                            sockets.remove(&id);
                        }
                    }) {
                    Ok(worker) => workers.push(worker),
                    Err(_) => {
                        if let Ok(mut sockets) = shared.sockets.lock() {
                            sockets.remove(&id);
                        }
                    },
                }
                let mut index = 0;
                while index < workers.len() {
                    if workers[index].is_finished() {
                        let _ = workers.swap_remove(index).join();
                    } else {
                        index += 1;
                    }
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10))
            },
            Err(_) => break,
        }
    }
    for worker in workers {
        let _ = worker.join();
    }
}

fn certificates() -> Result<(ServerConfig, Vec<u8>, String)> {
    let mut ca = CertificateParams::new(Vec::<String>::new())?;
    ca.distinguished_name
        .push(rcgen::DnType::CommonName, "Moonmail fixture CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let key = KeyPair::generate()?;
    let root = ca.self_signed(&key)?;
    let issuer = Issuer::new(ca, key);
    let mut leaf =
        CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into(), "::1".into()])?;
    leaf.distinguished_name
        .push(rcgen::DnType::CommonName, "localhost");
    leaf.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    leaf.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(7);
    leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let key = KeyPair::generate()?;
    let cert = leaf.signed_by(&key, &issuer)?;
    let config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()?
    .with_no_client_auth()
    .with_single_cert(
        vec![cert.der().clone()],
        PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
    )?;
    Ok((config, root.der().to_vec(), root.pem()))
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{
            io::{BufRead, BufReader},
            time::Instant,
        },
    };

    #[test]
    fn shutdown_interrupts_idle_sessions_and_releases_all_listeners() -> Result {
        let server = Server::start(Config::default())?;
        let ports = [
            server.endpoints.imap_tls,
            server.endpoints.smtp_tls,
            server.endpoints.imap_starttls,
            server.endpoints.smtp_starttls,
        ];
        let socket = TcpStream::connect(("127.0.0.1", server.endpoints.imap_starttls))?;
        socket.set_read_timeout(Some(Duration::from_secs(2)))?;
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        reader.read_line(&mut line)?;
        assert!(line.starts_with("* OK"));
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(2));
        line.clear();
        assert_eq!(reader.read_line(&mut line)?, 0);
        for port in ports {
            assert!(TcpListener::bind(("127.0.0.1", port)).is_ok());
        }
        Ok(())
    }
}
