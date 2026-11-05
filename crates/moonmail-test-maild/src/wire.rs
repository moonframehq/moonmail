use {
    crate::{Error, Result},
    rustls::{ServerConfig, ServerConnection, StreamOwned},
    std::{
        io::{self, BufRead, BufReader, Read, Write},
        net::TcpStream,
        sync::Arc,
    },
};

enum Stream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ServerConnection, TcpStream>>),
}
impl Read for Stream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(s) => s.read(buf),
            Self::Tls(s) => s.read(buf),
        }
    }
}
impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(s) => s.write(buf),
            Self::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(s) => s.flush(),
            Self::Tls(s) => s.flush(),
        }
    }
}

pub(crate) struct Wire {
    reader: Option<BufReader<Stream>>,
    pub encrypted: bool,
}
impl Wire {
    pub fn new(socket: TcpStream) -> Self {
        Self {
            reader: Some(BufReader::new(Stream::Plain(socket))),
            encrypted: false,
        }
    }
    fn reader(&mut self) -> Result<&mut BufReader<Stream>> {
        self.reader
            .as_mut()
            .ok_or(Error::Invalid("Closed connection"))
    }
    pub fn send(&mut self, bytes: impl AsRef<[u8]>) -> Result {
        let stream = self.reader()?.get_mut();
        stream.write_all(bytes.as_ref())?;
        stream.flush()?;
        Ok(())
    }
    pub fn line(&mut self, limit: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.reader()?
            .take(limit as u64 + 1)
            .read_until(b'\n', &mut bytes)?;
        if bytes.is_empty() {
            return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into());
        }
        if bytes.len() > limit || !bytes.ends_with(b"\r\n") {
            return Err(Error::Invalid("Invalid or oversized protocol line"));
        }
        Ok(bytes)
    }
    pub fn literal(&mut self, length: usize) -> Result<Vec<u8>> {
        if length > crate::store::MAX_MESSAGE {
            return Err(Error::Invalid("Oversized literal"));
        }
        let mut bytes = vec![0; length];
        self.reader()?.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    pub fn upgrade(&mut self, config: Arc<ServerConfig>) -> Result {
        let reader = self
            .reader
            .take()
            .ok_or(Error::Invalid("Closed connection"))?;
        if !reader.buffer().is_empty() {
            return Err(Error::Invalid("Pipelined data across STARTTLS"));
        }
        let Stream::Plain(socket) = reader.into_inner() else {
            return Err(Error::Invalid("Already encrypted"));
        };
        self.reader = Some(BufReader::new(Stream::Tls(Box::new(StreamOwned::new(
            ServerConnection::new(config)?,
            socket,
        )))));
        self.encrypted = true;
        Ok(())
    }
}
