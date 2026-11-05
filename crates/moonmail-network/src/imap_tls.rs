//! STARTTLS on the caller's timeout-bounded socket. rust-imap's connection
//! builder does not expose socket connect/write timeouts in the pinned version.
use {
    crate::{Error, Result},
    std::io::{BufRead, BufReader, Read, Write},
};

pub(crate) fn upgrade<T: Read + Write>(mut client: imap::Client<T>) -> Result<T> {
    if !client.capabilities()?.has_str("STARTTLS") {
        return Err(Error::Invalid(
            "IMAP server does not offer required STARTTLS",
        ));
    }
    let mut stream = client.into_inner()?;
    stream.write_all(b"MOONMAILTLS STARTTLS\r\n")?;
    stream.flush()?;
    // Never read ahead into the TLS handshake. Bound pre-TLS unsolicited lines.
    for _ in 0..64 {
        let mut line = Vec::new();
        BufReader::with_capacity(1, &mut stream)
            .take(8193)
            .read_until(b'\n', &mut line)?;
        if line.len() > 8192 || !line.ends_with(b"\r\n") {
            return Err(Error::Invalid("Invalid IMAP STARTTLS response"));
        }
        let (rest, response) = imap_proto::parser::parse_response(&line)
            .map_err(|_| Error::Invalid("Malformed IMAP STARTTLS response"))?;
        if !rest.is_empty() {
            return Err(Error::Invalid("Unexpected IMAP STARTTLS data"));
        }
        match response {
            imap_proto::Response::Done {
                tag,
                status: imap_proto::Status::Ok,
                ..
            } if tag.as_bytes() == b"MOONMAILTLS" => return Ok(stream),
            imap_proto::Response::Data {
                status: imap_proto::Status::Ok,
                ..
            } => {},
            _ => {
                return Err(Error::Invalid(
                    "IMAP STARTTLS was refused; no credentials were sent",
                ));
            },
        }
    }
    Err(Error::Invalid("Too many IMAP STARTTLS responses"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn starttls_requires_capability_and_success_and_never_reads_ahead()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        for (capability, accepted) in [(false, false), (true, false), (true, true)] {
            let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
            let address = listener.local_addr()?;
            let server = std::thread::spawn(move || -> std::io::Result<()> {
                let (mut socket, _) = listener.accept()?;
                socket.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
                let mut reader = BufReader::new(socket.try_clone()?);
                let mut command = String::new();
                reader.read_line(&mut command)?;
                let tag = command
                    .split_whitespace()
                    .next()
                    .ok_or_else(|| std::io::Error::other("missing tag"))?;
                assert!(command.ends_with(" CAPABILITY\r\n"));
                write!(
                    socket,
                    "* CAPABILITY IMAP4rev1 {}\r\n{tag} OK capabilities\r\n",
                    if capability { "STARTTLS" } else { "" }
                )?;
                command.clear();
                reader.read_line(&mut command)?;
                if !capability {
                    assert!(command.is_empty());
                    return Ok(());
                }
                assert_eq!(command, "MOONMAILTLS STARTTLS\r\n");
                if accepted {
                    socket.write_all(b"MOONMAILTLS OK begin TLS\r\nNEXT")?;
                } else {
                    socket.write_all(b"MOONMAILTLS NO refused\r\n")?;
                }
                Ok(())
            });
            let socket = std::net::TcpStream::connect(address)?;
            socket.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
            let result = upgrade(imap::Client::new(socket));
            assert_eq!(result.is_ok(), accepted);
            if let Ok(mut stream) = result {
                let mut bytes = [0; 4];
                stream.read_exact(&mut bytes)?;
                assert_eq!(&bytes, b"NEXT");
            }
            server.join().map_err(|_| "fake IMAP server panicked")??;
        }
        Ok(())
    }
}
