#[test]
fn wakes_for_relevant_account_changes_only() {
    let event = b": ping\n\nevent: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"a\":{\"Email\":\"s\"}}}\n\n";
    assert!(super::state_changed(&event[..], "a"));
    assert!(!super::state_changed(&event[..], "b"));
}
#[test]
fn skips_heartbeats_malformed_and_unrelated_events_before_mailbox_change() {
    let stream = concat!(
        ": heartbeat\n\n",
        "data: {broken}\n\n",
        "data: {\"@type\":\"StateChange\",\"changed\":{\"a\":{\"Thread\":\"t\"}}}\n\n",
        "event: state\ndata: {\"@type\":\"StateChange\",\n",
        "data: \"changed\":{\"a\":{\"Mailbox\":\"m\"}}}\n\n"
    );
    assert!(super::state_changed(stream.as_bytes(), "a"));
    assert!(!super::state_changed(&[0xff, b'\n'][..], "a"));
}

#[test]
fn paused_subscription_suppresses_late_mail_and_dropped_client_stops_reconnecting()
-> Result<(), Box<dyn std::error::Error>> {
    use {
        crate::jmap::Client,
        serde_json::json,
        std::{
            io::{BufRead, BufReader, Write},
            net::TcpListener,
            sync::{Arc, mpsc},
            thread,
            time::Duration,
        },
    };
    for pause in [true, false] {
        let socket = TcpListener::bind("127.0.0.1:0")?;
        let address = socket.local_addr()?;
        let (release, until_release) = mpsc::channel();
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut session, _) = socket.accept()?;
            for line in BufReader::new(&session).lines() {
                if line?.is_empty() {
                    break;
                }
            }
            let body=json!({"apiUrl":"/api","downloadUrl":"/blob","uploadUrl":"/upload","eventSourceUrl":"/push",
                "primaryAccounts":{"urn:ietf:params:jmap:mail":"a"},"capabilities":{"urn:ietf:params:jmap:core":{"maxObjectsInGet":2}}}).to_string();
            write!(
                session,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
            let (mut events, _) = socket.accept()?;
            for line in BufReader::new(&events).lines() {
                if line?.is_empty() {
                    break;
                }
            }
            events.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: text/event-stream\r\n\r\n: heartbeat\n\n")?;
            until_release
                .recv_timeout(Duration::from_secs(5))
                .map_err(std::io::Error::other)?;
            events.write_all(b"event: state\ndata: {\"@type\":\"StateChange\",\"changed\":{\"a\":{\"Mailbox\":\"s\"}}}\n\n")
        });
        let client = Arc::new(Client::discover(
            &format!("http://{address}/session"),
            "fixture".into(),
        )?);
        let (woke, wakes) = mpsc::channel();
        let mut interest = Some(super::listen(&client, move || {
            let _ = woke.send(());
        })?);
        wakes.recv_timeout(Duration::from_secs(5))?;
        if pause {
            drop(interest.take());
        }
        drop(client);
        release.send(())?;
        if !pause {
            wakes.recv_timeout(Duration::from_secs(5))?;
        }
        assert_eq!(
            wakes.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)
        );
        server.join().map_err(|_| "fixture panicked")??;
    }
    Ok(())
}
