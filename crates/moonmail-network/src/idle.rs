//! RFC 2177 shares the authenticated connection with rust-imap. The pinned
//! IDLE handle expects the continuation first and discards errors reading the
//! DONE acknowledgement. Servers can send mailbox updates at either boundary.
use {
    crate::{Error, Result},
    imap_proto::{MailboxDatum, Response, Status},
    native_tls::TlsStream,
    std::{
        io::{self, BufRead, BufReader, Read, Write},
        net::TcpStream,
        sync::{Arc, Mutex, MutexGuard},
        time::{Duration, Instant},
    },
};

type BufferedTls = BufReader<TlsStream<TcpStream>>;

// Both handles stay on the same worker. A shared buffer keeps read-ahead on
// the connection when switching between ordinary commands and IDLE.
#[derive(Clone)]
pub(crate) struct IdleStream(Arc<Mutex<BufferedTls>>);

impl IdleStream {
    pub fn new(tls: TlsStream<TcpStream>) -> Self {
        Self(Arc::new(Mutex::new(BufReader::new(tls))))
    }
    fn stream(&self) -> io::Result<MutexGuard<'_, BufferedTls>> {
        self.0
            .lock()
            .map_err(|_| io::Error::other("IMAP transport stopped"))
    }

    pub fn idle(&self, end: Instant, interrupted: impl Fn() -> bool) -> Result<bool> {
        let mut stream = self.stream()?;
        stream.get_mut().write_all(b"moonmail-idle IDLE\r\n")?;
        stream.get_mut().flush()?;
        let mut active = false;
        let mut terminating = false;
        let mut changed = false;
        let mut line = Vec::new();
        // Waiting for '+' and the acknowledged DONE are ordinary bounded commands.
        let command_timeout = Duration::from_secs(30);
        let mut deadline = Instant::now() + command_timeout;
        loop {
            if active && !terminating && (changed || interrupted() || Instant::now() >= end) {
                stream.get_mut().write_all(b"DONE\r\n")?;
                stream.get_mut().flush()?;
                terminating = true;
                deadline = Instant::now() + command_timeout;
                stream
                    .get_ref()
                    .get_ref()
                    .set_read_timeout(Some(command_timeout))?;
            }
            if (!active || terminating) && Instant::now() >= deadline {
                return Err(imap::Error::Io(io::ErrorKind::TimedOut.into()).into());
            }
            const LIMIT: usize = 1024 * 1024;
            let remaining = LIMIT.saturating_sub(line.len());
            if remaining == 0 {
                return Err(Error::Invalid("Oversized IMAP IDLE response"));
            }
            match (&mut *stream)
                .take(remaining as u64)
                .read_until(b'\n', &mut line)
            {
                Ok(0) => return Err(imap::Error::ConnectionLost.into()),
                Err(e)
                    if active
                        && !terminating
                        && matches!(
                            e.kind(),
                            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                        ) =>
                {
                    continue;
                },
                Err(e) => return Err(imap::Error::Io(e).into()),
                Ok(_) => {},
            }
            let response = match Response::from_bytes(&line) {
                Ok(([], response)) => response,
                Err(e) if e.is_incomplete() => continue,
                _ => return Err(Error::Invalid("Malformed IMAP IDLE response")),
            };
            match response {
                Response::Continue { .. } if !active => {
                    active = true;
                    stream
                        .get_ref()
                        .get_ref()
                        .set_read_timeout(Some(Duration::from_millis(150)))?;
                },
                Response::MailboxData(MailboxDatum::Exists(_))
                | Response::Expunge(_)
                | Response::Fetch(..) => changed = true,
                Response::Data {
                    status: Status::Bye,
                    ..
                } => return Err(imap::Error::ConnectionLost.into()),
                Response::Done {
                    tag,
                    status: Status::Ok,
                    ..
                } if terminating && tag.as_bytes() == b"moonmail-idle" => return Ok(changed),
                Response::Done { .. } | Response::Continue { .. } => {
                    return Err(Error::Invalid("Server rejected IMAP IDLE"));
                },
                _ => {},
            }
            line.clear();
        }
    }
}

impl Read for IdleStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        // rust-imap has its own BufStream. Never let it read beyond the final
        // tagged line and retain the next unsolicited update in its private buffer.
        if buf.is_empty() {
            return Ok(0);
        }
        let mut stream = self.stream()?;
        let available = stream.fill_buf()?;
        let chunk = &available[..available.len().min(buf.len())];
        let n = chunk
            .iter()
            .position(|&b| b == b'\n')
            .map_or(chunk.len(), |i| i + 1);
        buf[..n].copy_from_slice(&chunk[..n]);
        stream.consume(n);
        Ok(n)
    }
}
impl Write for IdleStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stream()?.get_mut().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.stream()?.get_mut().flush()
    }
}
