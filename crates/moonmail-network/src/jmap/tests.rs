use super::*;

#[test]
fn fastmail_endpoint_permissions_are_scoped_to_the_discovered_service()
-> std::result::Result<(), url::ParseError> {
    let origin = Url::parse(SESSION)?;
    for (endpoint, download, allowed) in [
        ("https://api.fastmail.com/jmap/api", false, true),
        ("https://jmap.fastmail.com/api", false, true),
        ("https://eu.api.fastmail.com/api", false, true),
        ("https://www.fastmailusercontent.com/mail", true, true),
        ("https://eu1-www.fastmailusercontent.com/mail", true, true),
        ("https://eu1-www.fastmailusercontent.com/mail", false, false),
        ("https://a.b-www.fastmailusercontent.com/mail", true, false),
        ("http://www.fastmailusercontent.com/mail", true, false),
        ("https://api.fastmail.com:8080/api", false, false),
        ("https://user@api.fastmail.com/api", false, false),
    ] {
        assert_eq!(
            validate_endpoint(&origin, &Url::parse(endpoint)?, download).is_ok(),
            allowed
        );
    }
    // Invalid header bytes are rejected by the HTTP request builder before DNS or I/O.
    assert!(matches!(
        Client::fastmail(SecretString::from("invalid\nheader")),
        Err(JmapError::Http(_))
    ));
    Ok(())
}
#[test]
fn streaming_limit_is_enforced_without_a_content_length()
-> std::result::Result<(), Box<dyn std::error::Error>> {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };
    let socket = TcpListener::bind("127.0.0.1:0")?;
    let address = socket.local_addr()?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = socket.accept()?;
        for line in BufReader::new(&stream).lines() {
            if line?.is_empty() {
                break;
            }
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\nbody\r\n0\r\n\r\n")
    });
    let reply = Http::new().get(format!("http://{address}/blob")).send()?;
    assert!(matches!(bounded(reply, 2), Err(JmapError::Protocol)));
    server.join().map_err(|_| "fixture panicked")??;
    Ok(())
}
