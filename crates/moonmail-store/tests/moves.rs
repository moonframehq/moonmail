use {
    moonmail_core::{MailQuery, MailRepository, MailboxRole},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage, MoveTarget},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn local_copies_preserve_source_destination_flags_and_batch_atomicity() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let source = archive.mailbox("local", "Source")?;
    let destination = archive.mailbox("local", "Destination")?;
    let other = archive.mailbox("other", "Other")?;
    let mut ids = Vec::new();
    for (account, mailbox) in [("local", source), ("other", other)] {
        ids.push(archive.ingest(
            &StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            IncomingMessage {
                account,
                mailbox,
                origin: "copy",
                epoch: 0,
                uid: None,
                seen: false,
                starred: true,
                received_at: 1,
                raw: b"From: test@example.org\r\nSubject: Copy\r\n\r\nOriginal",
            },
        )?);
    }
    assert!(
        archive
            .queue_copies(&ids, None, MoveTarget::Folder(destination))
            .is_err()
    );
    let query = |mailbox| MailQuery {
        mailbox: Some(mailbox),
        ..Default::default()
    };
    assert!(archive.page(&query(destination), 10)?.rows.is_empty());
    archive.queue_copies(
        &[ids[0], ids[0]],
        Some(source),
        MoveTarget::Folder(destination),
    )?;
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute(
        "UPDATE occurrences SET seen=1,flagged=0 WHERE mailbox_id=?1",
        [destination],
    )?;
    archive.queue_copies(&[ids[0]], Some(source), MoveTarget::Folder(destination))?;
    assert!(
        archive
            .queue_copies(&[ids[0]], None, MoveTarget::Folder(destination))
            .is_err(),
        "ambiguous origin requires explicit folder"
    );
    drop(archive);
    let archive = Archive::open(dir.path())?;
    let from = archive.page(&query(source), 10)?.rows;
    let to = archive.page(&query(destination), 10)?.rows;
    assert_eq!(from.len(), 1);
    assert_eq!(to.len(), 1);
    assert_eq!(from[0].id, to[0].id);
    assert!(from[0].unread && from[0].starred);
    let destination_flags: (bool, bool) = db.query_row(
        "SELECT seen,flagged FROM occurrences WHERE mailbox_id=?1",
        [destination],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    assert_eq!(destination_flags, (true, false));
    assert!(archive.move_operations()?.is_empty());
    Ok(())
}

#[test]
fn version_fifteen_upgrade_keeps_queued_operations_as_moves() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let source = archive.mailbox("a", "Source")?;
    let destination = archive.mailbox("a", "Destination")?;
    let id = archive.ingest(
        &StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        IncomingMessage {
            account: "a",
            mailbox: source,
            origin: "imap:copy",
            epoch: 7,
            uid: Some(1),
            seen: false,
            starred: false,
            received_at: 1,
            raw: b"From: test@example.org\r\n\r\nOriginal",
        },
    )?;
    archive.queue_moves(&[id], Some(source), MoveTarget::Folder(destination))?;
    drop(archive);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(concat!(
        include_str!("fixtures/remove_sent_copy.sql"),
        "DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules;"
    ))?;
    db.execute_batch("DROP TABLE occurrence_keywords; DROP TABLE keyword_labels; DROP TABLE keyword_operations; ALTER TABLE move_operations DROP COLUMN kind; PRAGMA user_version=15;")?;
    let mut archive = Archive::open(dir.path())?;
    let operation = archive.pending_moves("a")?.remove(0);
    assert_eq!(operation.kind, "move");
    assert!(archive.claim_move(operation.id)?);
    archive.finish_move(operation.id, "done", "acknowledged")?;
    assert!(archive.page(&MailQuery::default(), 10)?.rows.is_empty());
    Ok(())
}

#[test]
fn roles_require_explicit_or_unambiguous_server_identity_and_survive_list_changes() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let a = archive.mailbox("me@example.org", "Conserves")?;
    let b = archive.mailbox("me@example.org", "Personal/Old")?;
    archive.mailbox("me@example.org", "Archive")?;
    assert!(
        archive
            .role_mailbox("me@example.org", MailboxRole::Archive)
            .is_err()
    );
    archive.mailbox_server_role(a, Some(MailboxRole::Archive))?;
    assert_eq!(
        archive.role_mailbox("me@example.org", MailboxRole::Archive)?,
        a
    );
    archive.mailbox_server_role(b, Some(MailboxRole::Archive))?;
    assert!(
        archive
            .role_mailbox("me@example.org", MailboxRole::Archive)
            .is_err()
    );
    archive.set_mailbox_role(b, Some(MailboxRole::Archive))?;
    archive.mailbox_server_role(b, None)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert_eq!(
        archive.role_mailbox("me@example.org", MailboxRole::Archive)?,
        b
    );
    archive.reconcile_mailboxes("me@example.org", ["Conserves"])?;
    assert!(
        archive
            .role_mailbox("me@example.org", MailboxRole::Archive)
            .is_err(),
        "missing explicit mapping must not silently switch targets"
    );
    archive.set_mailbox_role(a, Some(MailboxRole::Archive))?;
    assert_eq!(
        archive.role_mailbox("me@example.org", MailboxRole::Archive)?,
        a
    );
    Ok(())
}

#[test]
fn bulk_moves_are_atomic_account_scoped_and_local_moves_preserve_message_identity() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let source = archive.mailbox("local", "Imported")?;
    let destination = archive.mailbox("local", "Filed")?;
    let other = archive.mailbox("other", "Elsewhere")?;
    let mut ids = Vec::new();
    for (origin, account, mailbox) in [("one", "local", source), ("two", "other", other)] {
        ids.push(archive.ingest(
            &StalwartParser::default(),
            &blobs,
            IncomingMessage {
                account,
                mailbox,
                origin,
                epoch: 0,
                uid: None,
                seen: false,
                starred: true,
                received_at: 1,
                raw: b"From: test@example.org\r\nSubject: Synthetic\r\n\r\nBody",
            },
        )?);
    }
    assert!(
        archive
            .queue_moves(&ids, None, MoveTarget::Folder(destination))
            .is_err()
    );
    assert_eq!(
        archive
            .page(
                &MailQuery {
                    mailbox: Some(source),
                    ..Default::default()
                },
                10
            )?
            .rows[0]
            .id,
        ids[0],
        "partial batch rolled back"
    );
    assert!(
        archive
            .page(
                &MailQuery {
                    mailbox: Some(destination),
                    ..Default::default()
                },
                10
            )?
            .rows
            .is_empty()
    );
    assert_eq!(
        archive.queue_moves(&[ids[0], ids[0]], None, MoveTarget::Folder(destination))?,
        1
    );
    assert_eq!(
        archive
            .page(
                &MailQuery {
                    mailbox: Some(destination),
                    ..Default::default()
                },
                10
            )?
            .rows[0]
            .id,
        ids[0]
    );
    assert!(archive.detail(ids[0])?.summary.starred);
    assert!(
        archive.move_operations()?.is_empty(),
        "local moves never enqueue IMAP work"
    );
    assert_eq!(
        archive.queue_moves(&[ids[0]], None, MoveTarget::Folder(destination))?,
        0
    );
    Ok(())
}
