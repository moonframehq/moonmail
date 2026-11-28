use super::*;

#[test]
fn permanent_deletion_revalidates_scope_and_never_replays_an_uncertain_write() -> Result {
    use {
        moonmail_mime::StalwartParser,
        moonmail_store::{DeletionTarget, IncomingMessage, JmapFolder},
    };
    for (case, expected) in [
        ("moved", "stale"),
        ("roleChanged", "stale"),
        ("missing", "done"),
        ("stateMismatch", "stale"),
        ("forbidden", "unsupported"),
        ("vanished", "done"),
        ("lost", "unknown"),
        ("badAck", "unknown"),
        ("badRead", "queued"),
        ("duplicate", "queued"),
        ("contradictory", "queued"),
    ] {
        let dir = tempfile::tempdir()?;
        let mut db = Archive::open(dir.path())?;
        let blobs = FileBlobs::new(dir.path());
        db.bind_jmap(ACCOUNT, "account")?;
        db.jmap_folders(
            ACCOUNT,
            &[JmapFolder {
                remote: "trash".into(),
                path: "Bin".into(),
                role: Some(MailboxRole::Junk),
            }],
            "f",
        )?;
        db.checkpoint_jmap(ACCOUNT, "s", None, &[], false)?;
        let bin = db.role_mailbox(ACCOUNT, MailboxRole::Junk)?;
        let uid = db.jmap_uid(ACCOUNT, "m")?;
        let id = db.ingest(
            &StalwartParser::default(),
            &blobs,
            IncomingMessage {
                account: ACCOUNT,
                mailbox: bin,
                origin: "jmap:m",
                epoch: 0,
                uid: Some(uid),
                seen: false,
                starred: false,
                received_at: 1,
                raw: MIME,
            },
        )?;
        db.queue_expunge(
            ACCOUNT,
            bin,
            "Bin",
            0,
            MailboxRole::Junk,
            DeletionTarget::Message(id, uid),
        )?;
        let writes = !matches!(case, "moved" | "roleChanged" | "missing") && expected != "queued";
        let error = matches!(expected, "unknown" | "queued");
        let (url, worker) = server(
            2 + usize::from(writes) + usize::from(!error) + usize::from(expected == "unknown"),
            move |n, base, r| {
                if n == 0 {
                    return session(base);
                }
                if n == 1 {
                    let body = r.json();
                    let calls = &body["methodCalls"];
                    assert_eq!(calls[0][0], "Mailbox/get");
                    assert_eq!(calls[1][1]["ids"], json!(["m"]));
                    let memberships = if case == "moved" {
                        json!({"in":true})
                    } else {
                        json!({"trash":true})
                    };
                    let (list, missing) = match case {
                        "missing" => (json!([]), json!(["m"])),
                        "badRead" => (json!([]), json!([])),
                        "duplicate" => (
                            json!([{"id":"m","mailboxIds":{"in":true,"trash":true}},{"id":"m","mailboxIds":{"trash":true}}]),
                            json!([]),
                        ),
                        "contradictory" => {
                            (json!([{"id":"m","mailboxIds":memberships}]), json!(["m"]))
                        },
                        _ => (json!([{"id":"m","mailboxIds":memberships}]), json!([])),
                    };
                    return json(
                        json!({"methodResponses":[response(&calls[0],json!({"list":[{"id":"trash","role":if case=="roleChanged" {"inbox"}else{"junk"}}]})),response(&calls[1],json!({"state":"before","list":list,"notFound":missing}))]}),
                    );
                }
                if n == 2 && writes {
                    let body = r.json();
                    let call = &body["methodCalls"][0];
                    assert_eq!(call[0], "Email/set");
                    assert_eq!(call[1]["ifInState"], "before");
                    assert_eq!(call[1]["destroy"], json!(["m"]));
                    return match case {
                        "lost" => Reply(503, vec![]),
                        "badAck" => method(&r, "Email/set", json!({})),
                        "stateMismatch" => json(
                            json!({"methodResponses":[["error",{"type":"stateMismatch"},call[2]]]}),
                        ),
                        _ => method(
                            &r,
                            "Email/set",
                            json!({"notDestroyed":{"m":{"type":if case=="vanished" {"notFound"}else{"forbidden"}}}}),
                        ),
                    };
                }
                delta(&r, "s", "s", vec![], vec![])
            },
        );
        let client = Client::discover(&url, "fixture".into())?;
        assert_eq!(
            client
                .sync(ACCOUNT, &mut db, &blobs, |_| {}, || false)
                .is_err(),
            error,
            "{case}"
        );
        drop(db);
        let mut db = Archive::open(dir.path())?;
        assert_eq!(db.remote_operations(None)?[0].state, expected, "{case}");
        if expected == "unknown" {
            db.jmap_folders(
                ACCOUNT,
                &[JmapFolder {
                    remote: "trash".into(),
                    path: "Renamed Junk".into(),
                    role: Some(MailboxRole::Junk),
                }],
                "f",
            )?;
            assert_eq!(
                db.queue_expunge(
                    ACCOUNT,
                    bin,
                    "Renamed Junk",
                    0,
                    MailboxRole::Junk,
                    DeletionTarget::Message(id, uid)
                )?,
                0
            );
            client.sync(ACCOUNT, &mut db, &blobs, |_| {}, || false)?;
        }
        worker.join().unwrap();
    }
    Ok(())
}

#[test]
fn cancelled_or_concurrently_claimed_deletions_cannot_issue_a_second_write() -> Result {
    use {
        moonmail_store::DeletionTarget,
        std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };
    for case in [
        "cancel_before_read",
        "cancel_after_read",
        "other_worker",
        "role_lost",
        "storage_error",
    ] {
        let root = tempfile::tempdir()?;
        let (mut db, id, inbox, _) = journal_fixture(root.path())?;
        db.mailbox_server_role(inbox, Some(MailboxRole::Trash))?;
        let uid = db.jmap_uid(ACCOUNT, "m")?;
        db.queue_expunge(
            ACCOUNT,
            inbox,
            "Inbox",
            0,
            MailboxRole::Trash,
            DeletionTarget::Message(id, uid),
        )?;
        let op = db.remote_operations(None)?[0].id;
        let path = root.path().to_owned();
        let read = Arc::new(AtomicBool::new(false));
        let mark = read.clone();
        let count = match case {
            "cancel_before_read" => 1,
            "cancel_after_read" | "storage_error" => 2,
            _ => 3,
        };
        let (url, w) = server(count, move |n, base, r| {
            if n == 0 {
                return session(base);
            }
            if n == 2 {
                return delta(&r, "s", "s", vec![], vec![]);
            }
            match case {
                "other_worker" => {
                    assert!(Archive::open(&path).unwrap().claim_expunge(op).unwrap());
                },
                "role_lost" => {
                    Archive::open(&path)
                        .unwrap()
                        .mailbox_server_role(inbox, None)
                        .unwrap();
                },
                "storage_error" => {
                    rusqlite::Connection::open(path.join("archive.sqlite3"))
                        .unwrap()
                        .execute_batch("ALTER TABLE mailboxes RENAME TO unavailable_folders")
                        .unwrap();
                },
                _ => {},
            }
            mark.store(true, Ordering::Release);
            let body = r.json();
            let calls = &body["methodCalls"];
            json(
                json!({"methodResponses":[response(&calls[0],json!({"list":[{"id":"in","role":"trash"}]})),
                response(&calls[1],json!({"state":"s","list":[{"id":"m","mailboxIds":{"in":true}}],"notFound":[]}))]}),
            )
        });
        let client = Client::discover(&url, "fixture".into())?;
        let first = std::cell::Cell::new(true);
        let result = client.sync(
            ACCOUNT,
            &mut db,
            &FileBlobs::new(root.path()),
            |_| {},
            || match case {
                "cancel_before_read" => !first.replace(false),
                "cancel_after_read" => read.load(Ordering::Acquire),
                _ => false,
            },
        );
        assert_eq!(
            result.is_err(),
            matches!(
                case,
                "cancel_before_read" | "cancel_after_read" | "storage_error"
            )
        );
        assert_eq!(
            db.remote_operations(None)?[0].state,
            match case {
                "other_worker" => "unknown",
                "role_lost" => "stale",
                _ => "queued",
            }
        );
        w.join().unwrap();
    }
    Ok(())
}
