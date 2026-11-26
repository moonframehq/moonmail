//! Scripted HTTP fixture. A malformed request should fail the fixture at its assertion site.
#![allow(clippy::unwrap_used)]

use {
    moonmail_core::{BlobStore, MailboxRole},
    moonmail_network::{
        SecretString, Submission, SubmissionFailure,
        jmap::{Client, JmapError},
    },
    moonmail_store::{Archive, FileBlobs},
    serde_json::{Value, json},
    std::{
        io::{BufRead, BufReader, Read, Write},
        net::{TcpListener, TcpStream},
        thread,
        time::Duration,
    },
};

const MIME:&[u8]=b"From: a@example.test\r\nTo: b@example.test\r\nSubject: Synthetic JMAP\r\nMessage-ID: <jmap@example.test>\r\n\r\nTest body\r\n";
const ACCOUNT: &str = "a@example.test";
type Result = std::result::Result<(), Box<dyn std::error::Error>>;
struct Request {
    path: String,
    body: Vec<u8>,
}
impl Request {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap()
    }
}
struct Reply(u16, Vec<u8>);
fn json(v: Value) -> Reply {
    Reply(200, v.to_string().into_bytes())
}
fn server(
    count: usize,
    mut serve: impl FnMut(usize, &str, Request) -> Reply + Send + 'static,
) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let url = format!("{base}/session");
    let thread = thread::spawn(move || {
        for n in 0..count {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(4)))
                .unwrap();
            respond(stream, |r| serve(n, &base, r));
        }
    });
    (url, thread)
}
fn respond(mut stream: TcpStream, serve: impl FnOnce(Request) -> Reply) {
    let mut input = BufReader::new(&stream);
    let mut line = String::new();
    input.read_line(&mut line).unwrap();
    let path = line.split_whitespace().nth(1).unwrap().to_owned();
    let mut len = 0;
    let mut auth = false;
    loop {
        line.clear();
        input.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        let (k, v) = line.split_once(':').unwrap();
        if k.eq_ignore_ascii_case("content-length") {
            len = v.trim().parse().unwrap();
        }
        if k.eq_ignore_ascii_case("authorization") {
            auth = v.trim() == "Bearer fixture";
        }
    }
    assert!(auth);
    let mut body = vec![0; len];
    input.read_exact(&mut body).unwrap();
    let Reply(code, bytes) = serve(Request { path, body });
    write!(
        stream,
        "HTTP/1.1 {code} Response\r\nConnection: close\r\nContent-Length: {}\r\n\r\n",
        bytes.len()
    )
    .unwrap();
    stream.write_all(&bytes).unwrap();
}
fn session(base: &str) -> Reply {
    json(json!({
        "apiUrl":format!("{base}/api"),"downloadUrl":format!("{base}/download/{{accountId}}/{{blobId}}/{{name}}"),
        "uploadUrl":format!("{base}/upload/{{accountId}}"),"eventSourceUrl":format!("{base}/events?types={{types}}&closeafter={{closeafter}}"),
        "primaryAccounts":{"urn:ietf:params:jmap:mail":"account"},"capabilities":{"urn:ietf:params:jmap:core":{"maxObjectsInGet":2}}
    }))
}
fn response(call: &Value, mut data: Value) -> Value {
    assert_eq!(call[1]["accountId"], "account");
    data["accountId"] = json!("account");
    json!([call[0], data, call[2]])
}
fn method(request: &Request, name: &str, data: Value) -> Reply {
    assert_eq!(request.path, "/api");
    let call = &request.json()["methodCalls"][0];
    assert_eq!(call[0], name);
    json(json!({"methodResponses":[response(call,data)]}))
}
fn email(folder: &str) -> Value {
    json!({"id":"message","blobId":"blob","size":MIME.len(),"receivedAt":"2025-01-01T00:00:00Z","keywords":{"project":true},"mailboxIds":{folder:true}})
}
fn folders() -> Value {
    json!({"state":"f","list":[{"id":"in","name":"Inbox","role":"inbox"},{"id":"sent","name":"Sent","role":"sent"},{"id":"drafts","name":"Drafts","role":"drafts"}]})
}
fn delta(
    request: &Request,
    old: &str,
    new: &str,
    updated: Vec<Value>,
    destroyed: Vec<&str>,
) -> Reply {
    let calls = &request.json()["methodCalls"];
    assert_eq!(calls[1][1]["sinceState"], old);
    assert_eq!(calls[3][1]["#ids"]["resultOf"], calls[1][2]);
    let changes = |from: &str, to: &str| json!({"oldState":from,"newState":to,"created":[],"updated":[],"destroyed":[],"hasMoreChanges":false});
    let mut messages = changes(old, new);
    messages["updated"] = json!(updated.iter().map(|v| &v["id"]).collect::<Vec<_>>());
    messages["destroyed"] = json!(destroyed);
    json(
        json!({"methodResponses":[response(&calls[0],changes("f","f")),response(&calls[1],messages),response(&calls[2],json!({"list":[],"notFound":[]})),response(&calls[3],json!({"list":updated,"notFound":[]}))]}),
    )
}
#[test]
fn initial_incremental_and_destroy_survive_reopen() -> Result {
    let (url, worker) = server(11, |n, base, r| match n {
        0 => session(base),
        1 => method(&r, "Mailbox/get", folders()),
        2 => method(&r, "Email/get", json!({"state":"s1","list":[]})),
        3 => method(
            &r,
            "Email/query",
            json!({"queryState":"query","position":0,"ids":["message"]}),
        ),
        4 => method(&r, "Email/get", json!({"list":[email("in")],"notFound":[]})),
        5 => {
            assert_eq!(r.path, "/download/account/blob/message.eml");
            Reply(200, MIME.into())
        },
        6 => {
            assert_eq!(r.json()["methodCalls"][0][1]["position"], 1);
            method(
                &r,
                "Email/query",
                json!({"queryState":"query","position":1,"ids":[]}),
            )
        },
        7 => delta(&r, "s1", "s1", vec![], vec![]),
        8 => {
            assert_eq!(
                r.json()["methodCalls"][0][1]["update"]["message"]["keywords/$seen"],
                true
            );
            method(&r, "Email/set", json!({"updated":{"message":null}}))
        },
        9 => delta(&r, "s1", "s2", vec![email("sent")], vec![]),
        _ => delta(&r, "s2", "s3", vec![], vec!["message"]),
    });
    let client = Client::discover(&url, "fixture".into())?;
    let dir = tempfile::tempdir()?;
    let blobs = FileBlobs::new(dir.path());
    let mut archive = Archive::open(dir.path())?;
    client.sync(ACCOUNT, &mut archive, &blobs, |_| {}, || false)?;
    let id = archive.jmap_message(ACCOUNT, "message")?.unwrap();
    assert_eq!(blobs.get(&archive.message_blob(id)?)?, MIME);
    archive.set_flag(id, "seen", true)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    client.sync(ACCOUNT, &mut archive, &blobs, |_| {}, || false)?;
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    let folder: i64 = db.query_row("SELECT mailbox_id FROM occurrences", [], |r| r.get(0))?;
    assert_eq!(folder, archive.role_mailbox(ACCOUNT, MailboxRole::Sent)?);
    client.sync(ACCOUNT, &mut archive, &blobs, |_| {}, || false)?;
    assert_eq!(
        db.query_row("SELECT count(*) FROM occurrences", [], |r| r
            .get::<_, i64>(0))?,
        0
    );
    assert_eq!(archive.jmap_states(ACCOUNT)?.1.as_deref(), Some("s3"));
    worker.join().unwrap();
    Ok(())
}
#[test]
fn submission_preserves_bytes_envelope_and_ambiguous_outcomes() -> Result {
    for uncertain in [false, true] {
        let (url, worker) = server(6, move |n, base, r| match n {
            0 => session(base),
            1 => method(
                &r,
                "Identity/get",
                json!({"list":[{"id":"identity","email":ACCOUNT}]}),
            ),
            2 => method(&r, "Mailbox/get", folders()),
            3 => {
                assert_eq!(r.path, "/upload/account");
                assert_eq!(r.body, MIME);
                json(json!({"accountId":"account","blobId":"upload","size":MIME.len()}))
            },
            4 => {
                assert_eq!(
                    r.json()["methodCalls"][0][1]["emails"]["message"]["blobId"],
                    "upload"
                );
                method(
                    &r,
                    "Email/import",
                    json!({"created":{"message":{"id":"draft"}}}),
                )
            },
            _ => {
                let call = &r.json()["methodCalls"][0];
                assert_eq!(
                    call[1]["create"]["send"]["envelope"]["rcptTo"][0]["email"],
                    "hidden@example.test"
                );
                assert_eq!(
                    call[1]["onSuccessUpdateEmail"]["#send"]["mailboxIds/sent"],
                    true
                );
                if uncertain {
                    Reply(200, b"{".into())
                } else {
                    method(
                        &r,
                        "EmailSubmission/set",
                        json!({"created":{"send":{"id":"submission"}}}),
                    )
                }
            },
        });
        let mut client = Client::discover(&url, "fixture".into())?;
        let outcome = client.submit(ACCOUNT, &["hidden@example.test".into()], MIME);
        if uncertain {
            assert_eq!(
                Client::failure(&outcome.unwrap_err()),
                SubmissionFailure::Unknown
            );
        } else {
            outcome?;
        }
        worker.join().unwrap();
    }
    Ok(())
}
#[test]
fn token_expiry_and_failed_writes_are_visible() -> Result {
    let (url, w) = server(1, |_, _, _| Reply(401, vec![]));
    assert!(matches!(
        Client::discover(&url, SecretString::from("fixture")),
        Err(JmapError::Unauthorized)
    ));
    w.join().unwrap();
    let (url, w) = server(3, |n, base, r| match n {
        0 => session(base),
        1 => {
            assert!(
                r.json()["methodCalls"][0][1]["update"]["m"]
                    .get("keywords/a~1b~0")
                    .is_some()
            );
            method(
                &r,
                "Email/set",
                json!({"notUpdated":{"m":{"type":"forbidden"}}}),
            )
        },
        _ => {
            assert!(
                r.path
                    .starts_with("/events?types=Mailbox,Email&closeafter=state")
            );
            Reply(200, b"event: state\n\n".into())
        },
    });
    let client = Client::discover(&url, "fixture".into())?;
    assert!(matches!(
        client.keyword("m", "a/b~", false),
        Err(JmapError::Rejected)
    ));
    let mut s = String::new();
    client.events()?.read_to_string(&mut s)?;
    assert!(s.starts_with("event: state"));
    w.join().unwrap();
    Ok(())
}

#[test]
fn missing_blob_leaves_checkpoint_replayable() -> Result {
    let (url, worker) = server(6, |n, base, r| match n {
        0 => session(base),
        1 => method(&r, "Mailbox/get", folders()),
        2 => method(&r, "Email/get", json!({"state":"initial"})),
        3 => method(
            &r,
            "Email/query",
            json!({"queryState":"query","position":0,"ids":["message"]}),
        ),
        4 => method(&r, "Email/get", json!({"list":[email("in")],"notFound":[]})),
        _ => Reply(200, b"truncated".into()),
    });
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let client = Client::discover(&url, "fixture".into())?;
    assert!(
        client
            .sync(
                ACCOUNT,
                &mut archive,
                &FileBlobs::new(dir.path()),
                |_| {},
                || false
            )
            .is_err()
    );
    drop(archive);
    assert!(Archive::open(dir.path())?.jmap_states(ACCOUNT)?.1.is_none());
    worker.join().unwrap();
    Ok(())
}

#[test]
fn mailbox_and_membership_edits_validate_acknowledgements() -> Result {
    let (url, worker) = server(8, |n, base, r| match n {
        0 => Reply(404, vec![]),
        1 => {
            assert_eq!(r.path, "/.well-known/jmap");
            session(base)
        },
        2 => {
            assert_eq!(
                r.json()["methodCalls"][0][1]["create"]["new"]["parentId"],
                "p"
            );
            method(
                &r,
                "Mailbox/set",
                json!({"created":{"new":{"id":"folder"}}}),
            )
        },
        3 => {
            assert_eq!(
                r.json()["methodCalls"][0][1]["update"]["folder"]["name"],
                "Later"
            );
            method(&r, "Mailbox/set", json!({"updated":{"folder":null}}))
        },
        4 => {
            assert_eq!(
                r.json()["methodCalls"][0][1]["update"]["message"],
                json!({"mailboxIds/folder":true,"mailboxIds/in":null})
            );
            method(&r, "Email/set", json!({"updated":{"message":null}}))
        },
        5 => {
            assert_eq!(r.json()["methodCalls"][0][1]["destroy"], json!(["message"]));
            method(&r, "Email/set", json!({"destroyed":["message"]}))
        },
        6 => method(&r, "Mailbox/set", json!({"destroyed":["folder"]})),
        _ => method(&r, "Email/set", json!({"updated":{"other":null}})),
    });
    let client = Client::discover(&url, "fixture".into())?;
    let id = client.create_mailbox("New", Some("p"))?;
    client.rename_mailbox(&id, "Later", None)?;
    client.transfer("message", Some("in"), &id)?;
    client.destroy_email("message")?;
    client.delete_mailbox(&id)?;
    assert!(matches!(
        client.keyword("message", "$flagged", true),
        Err(JmapError::Protocol)
    ));
    worker.join().unwrap();
    Ok(())
}

#[test]
fn upgrade_keeps_existing_archive_and_transport() -> Result {
    use moonmail_network::{Account, Transport};
    let dir = tempfile::tempdir()?;
    let original = Account::fastmail(ACCOUNT)?;
    let mut legacy = serde_json::to_value(original)?;
    legacy.as_object_mut().unwrap().remove("transport");
    std::fs::write(
        dir.path().join("account.json"),
        serde_json::to_vec(&legacy)?,
    )?;
    assert_eq!(
        Account::load(dir.path())?.unwrap().transport,
        Transport::Imap
    );
    let archive = Archive::open(dir.path())?;
    let mailbox = archive.mailbox(ACCOUNT, "Old mailbox")?;
    drop(archive);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch("DROP TABLE jmap_accounts; DROP TABLE jmap_emails; DROP TABLE jmap_folders; PRAGMA user_version=23;")?;
    let archive = Archive::open(dir.path())?;
    assert_eq!(archive.mailbox(ACCOUNT, "Old mailbox")?, mailbox);
    archive.bind_jmap(ACCOUNT, "a")?;
    assert_eq!(archive.jmap_states(ACCOUNT)?, (None, None));
    assert!(archive.bind_jmap(ACCOUNT, "b").is_err());
    Ok(())
}

#[test]
fn expired_state_recovers_with_new_baseline() -> Result {
    let (url, worker) = server(6, |n, base, r| match n {
        0 => session(base),
        1 => {
            json(json!({"methodResponses":[["error",{"type":"cannotCalculateChanges"},"folders"]]}))
        },
        2 => method(&r, "Mailbox/get", folders()),
        3 => method(&r, "Email/get", json!({"state":"recovered"})),
        4 => method(
            &r,
            "Email/query",
            json!({"queryState":"q","position":0,"ids":[]}),
        ),
        _ => delta(&r, "recovered", "recovered", vec![], vec![]),
    });
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    archive.bind_jmap(ACCOUNT, "account")?;
    archive.jmap_folders(ACCOUNT, &[], "expired-folders")?;
    archive.checkpoint_jmap(ACCOUNT, "expired-mail", None, &[], false)?;
    let client = Client::discover(&url, "fixture".into())?;
    client.sync(
        ACCOUNT,
        &mut archive,
        &FileBlobs::new(dir.path()),
        |_| {},
        || false,
    )?;
    assert_eq!(
        archive.jmap_states(ACCOUNT)?.1.as_deref(),
        Some("recovered")
    );
    worker.join().unwrap();
    Ok(())
}
