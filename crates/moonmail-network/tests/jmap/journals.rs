use super::*;

#[test]
fn folder_failures_and_stale_move_targets_remain_visible() -> Result {
    use moonmail_store::{FolderEdit, MoveTarget};
    for case in [
        "nested",
        "rejected",
        "lost",
        "folderChanged",
        "sourceMoved",
        "destinationGone",
    ] {
        let root = tempfile::tempdir()?;
        let (mut db, id, inbox, saved) = journal_fixture(root.path())?;
        let sql = rusqlite::Connection::open(root.path().join("archive.sqlite3"))?;
        match case {
            "folderChanged" => {
                db.queue_folder_edit(ACCOUNT, FolderEdit::Delete(saved))?;
                sql.execute("UPDATE mailboxes SET name='Changed' WHERE id=?1", [saved])?;
            },
            "sourceMoved" | "destinationGone" => {
                db.queue_moves(&[id], Some(inbox), MoveTarget::Folder(saved))?;
                if case == "destinationGone" {
                    sql.execute("UPDATE mailboxes SET active=0 WHERE id=?1", [saved])?;
                }
            },
            _ => {
                db.queue_folder_edit(ACCOUNT, FolderEdit::Create("Saved/Child".into()))?;
            },
        }
        let count = match case {
            "nested" | "sourceMoved" => 3,
            _ => 2,
        };
        let (url, w) = server(count, move |n, base, r| {
            if n == 0 {
                return session(base);
            }
            if case == "destinationGone" || case == "folderChanged" || n == 2 {
                return delta(&r, "s", "s", vec![], vec![]);
            }
            if case == "sourceMoved" {
                return method(
                    &r,
                    "Email/get",
                    json!({"state":"s","list":[{"id":"m","mailboxIds":{"elsewhere":true}}]}),
                );
            }
            assert_eq!(
                r.json()["methodCalls"][0][1]["create"]["new"]["parentId"],
                "saved"
            );
            match case {
                "lost" => Reply(503, vec![]),
                "rejected" => method(
                    &r,
                    "Mailbox/set",
                    json!({"notCreated":{"new":{"type":"forbidden"}}}),
                ),
                _ => method(&r, "Mailbox/set", json!({"created":{"new":{"id":"child"}}})),
            }
        });
        let client = Client::discover(&url, "fixture".into())?;
        let outcome = client.sync(
            ACCOUNT,
            &mut db,
            &FileBlobs::new(root.path()),
            |_| {},
            || false,
        );
        assert_eq!(
            outcome.is_err(),
            matches!(case, "lost" | "rejected"),
            "{case}"
        );
        let table = if matches!(case, "sourceMoved" | "destinationGone") {
            "move_operations"
        } else {
            "folder_operations"
        };
        let state: String =
            sql.query_row(&format!("SELECT state FROM {table}"), [], |r| r.get(0))?;
        assert_eq!(
            state,
            match case {
                "lost" => "unknown",
                "nested" => "done",
                _ => "failed",
            },
            "{case}"
        );
        w.join().unwrap();
    }
    Ok(())
}

#[test]
fn keyword_failures_keep_the_local_intent_and_do_not_block_later_sync() -> Result {
    for case in ["missingMembership", "denied", "offline"] {
        let root = tempfile::tempdir()?;
        let (mut db, id, _, _) = journal_fixture(root.path())?;
        db.set_keywords(&[id], "label", true)?;
        if case == "missingMembership" {
            rusqlite::Connection::open(root.path().join("archive.sqlite3"))?
                .execute("DELETE FROM occurrences WHERE message_id=?1", [id.0 as i64])?;
        }
        let (url, w) = server(
            if case == "offline" {
                2
            } else if case == "denied" {
                4
            } else {
                3
            },
            move |n, base, r| {
                if n == 0 {
                    return session(base);
                }
                if n == 1 && case != "missingMembership" {
                    if case == "offline" {
                        return Reply(503, vec![]);
                    }
                    return method(
                        &r,
                        "Email/set",
                        json!({"notUpdated":{"m":{"type":"forbidden"}}}),
                    );
                }
                delta(&r, "s", "s", vec![], vec![])
            },
        );
        let client = Client::discover(&url, "fixture".into())?;
        let blobs = FileBlobs::new(root.path());
        assert_eq!(
            client
                .sync(ACCOUNT, &mut db, &blobs, |_| {}, || false)
                .is_err(),
            case == "offline"
        );
        assert_eq!(
            db.keyword_operations(ACCOUNT, 0)?[0].state,
            if case == "offline" {
                "queued"
            } else {
                "blocked"
            }
        );
        if case != "offline" {
            client.sync(ACCOUNT, &mut db, &blobs, |_| {}, || false)?;
        }
        w.join().unwrap();
    }
    Ok(())
}

#[test]
fn server_side_errors_keep_the_correct_journal_state() -> Result {
    for case in [
        "createError",
        "moveRead",
        "wrongEmail",
        "otherMethod",
        "downloadHttp",
    ] {
        let root = tempfile::tempdir()?;
        let (mut db, id, inbox, saved) = journal_fixture(root.path())?;
        if matches!(case, "moveRead" | "wrongEmail") {
            db.queue_moves(
                &[id],
                Some(inbox),
                moonmail_store::MoveTarget::Folder(saved),
            )?;
        }
        let (url, w) = server(
            if case == "downloadHttp" { 3 } else { 2 },
            move |n, base, r| match n {
                0 => session(base),
                1 if case == "downloadHttp" => {
                    let mut m = email("in");
                    m["id"] = json!("new");
                    delta(&r, "s", "s2", vec![m], vec![])
                },
                _ if case == "otherMethod" => {
                    let body = r.json();
                    json(
                        json!({"methodResponses":[["error",{"type":"invalidArguments"},body["methodCalls"][0][2]]]}),
                    )
                },
                _ if case == "wrongEmail" => method(
                    &r,
                    "Email/get",
                    json!({"state":"s","list":[{"id":"unrequested","mailboxIds":{"in":true}}]}),
                ),
                _ => Reply(503, vec![]),
            },
        );
        let client = Client::discover(&url, "fixture".into())?;
        match case {
            "createError" => assert!(client.create_mailbox("nested", None).is_err()),
            "otherMethod" => assert!(matches!(
                client.destroy_email("m"),
                Err(JmapError::Rejected)
            )),
            _ => assert!(
                client
                    .sync(
                        ACCOUNT,
                        &mut db,
                        &FileBlobs::new(root.path()),
                        |_| {},
                        || false
                    )
                    .is_err()
            ),
        }
        if matches!(case, "moveRead" | "wrongEmail") {
            assert_eq!(db.pending_moves(ACCOUNT)?.len(), 1);
        }
        assert_eq!(db.jmap_states(ACCOUNT)?.1.as_deref(), Some("s"));
        w.join().unwrap();
    }
    Ok(())
}

#[test]
fn imap_journals_are_skipped_and_cancellation_keeps_new_jmap_intents_queued() -> Result {
    let root = tempfile::tempdir()?;
    let (mut db, id, inbox, saved) = journal_fixture(root.path())?;
    db.set_keywords(&[id], "local", true)?;
    db.queue_moves(
        &[id],
        Some(inbox),
        moonmail_store::MoveTarget::Folder(saved),
    )?;
    let sql = rusqlite::Connection::open(root.path().join("archive.sqlite3"))?;
    sql.execute_batch(
        "UPDATE keyword_operations SET epoch=93; UPDATE move_operations SET epoch=93;",
    )?;
    let (url, w) = server(2, |n, base, r| {
        if n == 0 {
            session(base)
        } else {
            delta(&r, "s", "s", vec![], vec![])
        }
    });
    let client = Client::discover(&url, "fixture".into())?;
    let blobs = FileBlobs::new(root.path());
    client.sync(ACCOUNT, &mut db, &blobs, |_| {}, || false)?;
    assert_eq!(db.pending_moves(ACCOUNT)?.len(), 1);
    sql.execute_batch("UPDATE keyword_operations SET epoch=0")?;
    let first = std::cell::Cell::new(true);
    assert!(matches!(
        client.sync(ACCOUNT, &mut db, &blobs, |_| {}, || !first.replace(false)),
        Err(moonmail_network::Error::Cancelled)
    ));
    assert_eq!(db.keyword_operations(ACCOUNT, 0)?[0].state, "queued");
    w.join().unwrap();
    Ok(())
}

#[test]
fn competing_workers_cannot_repeat_claimed_moves_or_folder_edits() -> Result {
    use moonmail_store::{FolderEdit, MoveTarget};
    for is_move in [true, false] {
        let root = tempfile::tempdir()?;
        let (mut db, id, inbox, saved) = journal_fixture(root.path())?;
        let op = if is_move {
            db.queue_moves(&[id], Some(inbox), MoveTarget::Folder(saved))?;
            db.pending_moves(ACCOUNT)?[0].id
        } else {
            db.queue_folder_edit(ACCOUNT, FolderEdit::Create("New".into()))?;
            db.pending_folder_edit(ACCOUNT)?.unwrap().id
        };
        let competing = Archive::open(root.path())?;
        let (url, server) = server(if is_move { 3 } else { 2 }, move |n, base, r| match n {
            0 => session(base),
            1 if is_move => method(
                &r,
                "Email/get",
                json!({"state":"s","list":[{"id":"m","mailboxIds":{"in":true}}]}),
            ),
            _ => delta(&r, "s", "s", vec![], vec![]),
        });
        let boundaries = std::cell::Cell::new(0);
        let client = Client::discover(&url, "fixture".into())?;
        client.sync(
            ACCOUNT,
            &mut db,
            &FileBlobs::new(root.path()),
            |_| {},
            || {
                let boundary = boundaries.get() + 1;
                boundaries.set(boundary);
                // The second boundary falls between reading and claiming the queued operation.
                if boundary == 2 {
                    assert!(if is_move {
                        competing.claim_move(op).unwrap()
                    } else {
                        competing.claim_folder_edit(op).unwrap()
                    });
                }
                false
            },
        )?;
        let sql = rusqlite::Connection::open(root.path().join("archive.sqlite3"))?;
        let query = if is_move {
            "SELECT state FROM move_operations WHERE id=?1"
        } else {
            "SELECT state FROM folder_operations WHERE id=?1"
        };
        let state: String = sql.query_row(query, [op], |r| r.get(0))?;
        assert_eq!(state, "running");
        server.join().unwrap();
    }
    Ok(())
}
