use super::*;

#[test]
fn invalid_initial_pages_leave_a_replayable_inventory() -> Result {
    for case in [
        "cycle",
        "parent",
        "position",
        "oversized",
        "missing",
        "pageChanged",
        "declaredSize",
    ] {
        let count = match case {
            "cycle" | "parent" => 2,
            "pageChanged" => 6,
            "declaredSize" => 5,
            _ => 4,
        };
        let (url, w) = server(count, move |n, base, r| match n {
            0 => session(base),
            1 => {
                let list = match case {
                    "cycle" => json!([{"id":"in","name":"Box","parentId":"in"}]),
                    "parent" => json!([{"id":"in","name":"Box","parentId":"absent"}]),
                    _ => folders()["list"].clone(),
                };
                method(&r, "Mailbox/get", json!({"state":"f","list":list}))
            },
            2 => method(&r, "Email/get", json!({"state":"s"})),
            3 => {
                let mut record = email("in");
                if case == "oversized" {
                    record["size"] = json!(64 * 1024 * 1024 + 1);
                }
                if case == "declaredSize" {
                    record["size"] = json!(1);
                }
                let body = r.json();
                let calls = &body["methodCalls"];
                json(
                    json!({"methodResponses":[response(&calls[0],json!({"ids":["message"],"queryState":"q","position":if case=="position"{1}else{0}})),response(&calls[1],json!({"list":if case=="missing" {json!([])}else{json!([record])},"notFound":[]}))]}),
                )
            },
            4 => Reply(200, MIME.into()),
            _ => {
                let body = r.json();
                let calls = &body["methodCalls"];
                json(
                    json!({"methodResponses":[response(&calls[0],json!({"ids":[],"queryState":"different","position":1})),response(&calls[1],json!({"list":[],"notFound":[]}))]}),
                )
            },
        });
        let root = tempfile::tempdir()?;
        let client = Client::discover(&url, "fixture".into())?;
        let mut db = Archive::open(root.path())?;
        assert!(
            client
                .sync(
                    ACCOUNT,
                    &mut db,
                    &FileBlobs::new(root.path()),
                    |_| {},
                    || false
                )
                .is_err(),
            "{case}"
        );
        assert_eq!(db.jmap_states(ACCOUNT)?.1, None, "{case}");
        w.join().unwrap();
    }
    Ok(())
}

#[test]
fn successive_delta_pages_ignore_unread_counts_and_reject_a_stalled_cursor() -> Result {
    let (url, worker) = server(4, |n, base, r| {
        if n == 0 {
            return session(base);
        }
        let body = r.json();
        let calls = &body["methodCalls"];
        let changes = |old: &str, new: &str, more: bool| json!({"oldState":old,"newState":new,"hasMoreChanges":more,"created":[],"updated":[],"destroyed":[]});
        let mut folders = changes(if n == 1 { "f" } else { "f2" }, "f2", false);
        folders["updatedProperties"] = json!(["unreadEmails", "totalThreads"]);
        folders["updated"] = json!(["t"]);
        let mail = match n {
            1 => changes("s", "s2", true),
            2 => changes("s2", "s3", false),
            _ => changes("s3", "s3", true),
        };
        json(
            json!({"methodResponses":[response(&calls[0],folders),response(&calls[1],mail),
            response(&calls[2],json!({"list":[],"notFound":[]})),response(&calls[3],json!({"list":[],"notFound":[]}))]}),
        )
    });
    let root = tempfile::tempdir()?;
    let mut db = Archive::open(root.path())?;
    let blobs = FileBlobs::new(root.path());
    let client = Client::discover(&url, "fixture".into())?;
    db.bind_jmap(ACCOUNT, "account")?;
    db.jmap_folders(ACCOUNT, &[], "f")?;
    db.checkpoint_jmap(ACCOUNT, "s", None, &[], false)?;
    client.sync(ACCOUNT, &mut db, &blobs, |_| {}, || false)?;
    assert_eq!(
        db.jmap_states(ACCOUNT)?,
        (Some("f2".into()), Some("s3".into()))
    );
    assert!(
        client
            .sync(ACCOUNT, &mut db, &blobs, |_| {}, || false)
            .is_err()
    );
    assert_eq!(db.jmap_states(ACCOUNT)?.1.as_deref(), Some("s3"));
    assert!(matches!(
        client.sync(ACCOUNT, &mut db, &blobs, |_| {}, || true),
        Err(moonmail_network::Error::Cancelled)
    ));
    worker.join().unwrap();
    Ok(())
}

#[test]
fn failed_ingest_and_checkpoint_keep_the_previous_page_available_for_replay() -> Result {
    use moonmail_store::JmapFolder;
    for case in [
        "blobStore",
        "metadata",
        "createdReply",
        "updatedReply",
        "checkpoint",
        "folders",
    ] {
        let root = tempfile::tempdir()?;
        let mut db = Archive::open(root.path())?;
        db.bind_jmap(ACCOUNT, "account")?;
        db.jmap_folders(
            ACCOUNT,
            &[JmapFolder {
                remote: "in".into(),
                path: "Inbox".into(),
                role: Some(MailboxRole::Inbox),
            }],
            "f",
        )?;
        if case != "folders" {
            db.checkpoint_jmap(ACCOUNT, "s", None, &[], false)?;
        }
        let sql = rusqlite::Connection::open(root.path().join("archive.sqlite3"))?;
        match case{
            "metadata"=>sql.execute_batch("CREATE TRIGGER fail_ingest BEFORE INSERT ON messages BEGIN SELECT RAISE(ABORT,'fixture disk error'); END;")?,
            "blobStore"=>std::fs::write(root.path().join("messages"),b"blocked shard directory")?,
            "checkpoint"=>sql.execute_batch("CREATE TRIGGER fail_state BEFORE UPDATE ON jmap_accounts BEGIN SELECT RAISE(ABORT,'fixture commit error'); END;")?,
            _=>{}
        }
        let (url, w) = server(
            if matches!(case, "metadata" | "blobStore") {
                3
            } else {
                2
            },
            move |n, base, r| {
                if n == 0 {
                    return session(base);
                }
                if n == 2 {
                    return Reply(200, MIME.into());
                }
                if case == "folders" {
                    return Reply(503, vec![]);
                }
                let body = r.json();
                let c = &body["methodCalls"];
                let create = matches!(case, "metadata" | "blobStore" | "createdReply");
                let changes = |old: &str, new: &str| json!({"oldState":old,"newState":new,"created":[],"updated":[],"destroyed":[],"hasMoreChanges":false});
                let mut mail = changes("s", "s2");
                if create {
                    mail["created"] = json!(["message"]);
                }
                if case == "updatedReply" {
                    mail["updated"] = json!(["message"]);
                }
                json(
                    json!({"methodResponses":[response(&c[0],changes("f","f")),response(&c[1],mail),
                response(&c[2],json!({"list":if create && case!="createdReply" {json!([email("in")])} else {json!([])},"notFound":[]})),
                response(&c[3],json!({"list":[],"notFound":[]}))]}),
                )
            },
        );
        let client = Client::discover(&url, "fixture".into())?;
        assert!(
            client
                .sync(
                    ACCOUNT,
                    &mut db,
                    &FileBlobs::new(root.path()),
                    |_| {},
                    || false
                )
                .is_err()
        );
        assert_eq!(
            db.jmap_states(ACCOUNT)?.1.as_deref(),
            if case == "folders" { None } else { Some("s") }
        );
        w.join().unwrap();
    }
    Ok(())
}

#[test]
fn mixed_download_page_reuses_known_blobs_and_commits_only_after_both_new_blobs_arrive() -> Result {
    let root = tempfile::tempdir()?;
    let (mut db, id, _, _) = journal_fixture(root.path())?;
    let (url, worker) = server(4, |n, base, r| {
        if n == 0 {
            let mut body: Value = serde_json::from_slice(&session(base).1).unwrap();
            body["capabilities"]["urn:ietf:params:jmap:core"]["maxObjectsInGet"] = json!(3);
            body["capabilities"]["urn:ietf:params:jmap:core"]["maxConcurrentRequests"] = json!(3);
            return json(body);
        }
        if n > 1 {
            assert!(r.path.contains("/new"));
            return Reply(200, MIME.into());
        }
        let mut records = Vec::new();
        for id in ["new1", "m", "new2"] {
            let mut record = email("in");
            record["id"] = json!(id);
            record["blobId"] = json!(id);
            records.push(record);
        }
        delta(&r, "s", "s2", records, vec![])
    });
    let client = Client::discover(&url, "fixture".into())?;
    client.sync(
        ACCOUNT,
        &mut db,
        &FileBlobs::new(root.path()),
        |_| {},
        || false,
    )?;
    assert_eq!(db.total()?, 3);
    assert_eq!(db.jmap_message(ACCOUNT, "m")?, Some(id));
    worker.join().unwrap();
    Ok(())
}
