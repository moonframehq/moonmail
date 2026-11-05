use {
    moonmail_core::{FolderChange, MailQuery, MailRepository, MailboxRole},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, FolderEdit, IncomingMessage, MoveTarget},
    rusqlite::Connection,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn local_hierarchy_edits_preserve_ids_roles_and_raw_mail_and_delete_only_named_leaves() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let parent = archive.mailbox("local", "Plans_%")?;
    archive.mailbox_hierarchy(parent, Some("."), false)?;
    let child = archive.mailbox("local", "Plans_%.Review")?;
    archive.mailbox_hierarchy(child, Some("."), true)?;
    let sibling = archive.mailbox("local", "PlansXY.Review")?;
    let other = archive.mailbox("other", "Plans_%.Review")?;
    archive.set_mailbox_role(child, Some(MailboxRole::Archive))?;
    let id = archive.ingest(
        &StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        IncomingMessage {
            account: "local",
            mailbox: child,
            origin: "synthetic",
            epoch: 0,
            uid: None,
            seen: false,
            starred: true,
            received_at: 1,
            raw: b"From: peer@example.org\r\nSubject: Preserved\r\n\r\nSynthetic original",
        },
    )?;
    let original = archive.detail(id)?.blob;
    assert!(
        archive
            .edit_local_folder("local", FolderEdit::Delete(parent))
            .is_err()
    );
    archive.edit_local_folder(
        "local",
        FolderEdit::Rename {
            id: parent,
            name: "Renamed".into(),
        },
    )?;
    let folders = archive.mailboxes()?;
    assert!(
        folders
            .iter()
            .any(|folder| folder.id == parent && folder.name == "Renamed" && !folder.selectable)
    );
    assert!(
        folders
            .iter()
            .any(|folder| folder.id == child && folder.name == "Renamed.Review")
    );
    assert!(
        folders
            .iter()
            .any(|folder| folder.id == sibling && folder.name == "PlansXY.Review")
    );
    assert!(
        folders
            .iter()
            .any(|folder| folder.id == other && folder.name == "Plans_%.Review")
    );
    assert_eq!(archive.role_mailbox("local", MailboxRole::Archive)?, child);
    archive.edit_local_folder("local", FolderEdit::Create("New".into()))?;
    let new = archive
        .mailboxes()?
        .into_iter()
        .find(|folder| folder.name == "New")
        .ok_or("missing new folder")?;
    assert_eq!(new.delimiter.as_deref(), Some("."));
    archive.edit_local_folder("local", FolderEdit::Create("New.Child".into()))?;
    assert!(
        archive
            .edit_local_folder("local", FolderEdit::Delete(new.id))
            .is_err(),
        "created folder inherits hierarchy semantics"
    );
    assert!(
        archive
            .edit_local_folder(
                "local",
                FolderEdit::Rename {
                    id: parent,
                    name: "New".into()
                }
            )
            .is_err()
    );
    for name in ["", "\r\nInjected", "INBOX", "Trailing."] {
        assert!(
            archive
                .edit_local_folder("local", FolderEdit::Create(name.into()))
                .is_err()
        );
    }
    archive.edit_local_folder("local", FolderEdit::Delete(child))?;
    archive.edit_local_folder("local", FolderEdit::Delete(parent))?;
    assert!(archive.role_mailbox("local", MailboxRole::Archive).is_err());
    assert!(archive.page(&MailQuery::default(), 10)?.rows.is_empty());
    assert_eq!(archive.detail(id)?.blob, original);
    assert_eq!(archive.detail(id)?.text, "Synthetic original");
    assert!(archive.folder_operations()?.is_empty());
    drop(archive);
    let archive = Archive::open(dir.path())?;
    assert!(!archive.mailboxes()?.iter().any(|folder| folder.id == child));
    assert_eq!(archive.detail(id)?.blob, original);
    Ok(())
}

#[test]
fn queued_rename_freezes_members_and_failed_local_commit_recovers_unknown_without_partial_names()
-> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let parent = archive.mailbox("a", "Projects")?;
    let child = archive.mailbox("a", "Projects/Review")?;
    archive.reconcile(parent, 7, &[])?;
    archive.reconcile(child, 8, &[])?;
    let id = archive.ingest(
        &StalwartParser::default(),
        &FileBlobs::new(dir.path()),
        IncomingMessage {
            account: "a",
            mailbox: child,
            origin: "imap:synthetic",
            epoch: 8,
            uid: Some(1),
            seen: true,
            starred: false,
            received_at: 1,
            raw: b"From: peer@example.org\r\n\r\nSynthetic",
        },
    )?;
    assert!(
        archive
            .edit_local_folder("a", FolderEdit::Create("Unsafe local".into()))
            .is_err()
    );
    let operation = archive.queue_folder_edit(
        "a",
        FolderEdit::Rename {
            id: parent,
            name: "Work".into(),
        },
    )?;
    assert!(
        archive
            .queue_moves(&[id], None, MoveTarget::Folder(parent))
            .is_err()
    );
    assert!(
        archive
            .queue_folder_edit("a", FolderEdit::Create("Another".into()))
            .is_err()
    );
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    let frozen = archive
        .pending_folder_edit("a")?
        .ok_or("missing durable intent")?;
    assert_eq!(frozen.id, operation);
    assert_eq!(frozen.members.len(), 2);
    assert!(
        matches!(frozen.change,FolderChange::Rename{ref source,ref destination} if source=="Projects" && destination=="Work")
    );
    assert!(archive.claim_folder_edit(operation)?);
    assert!(!archive.claim_folder_edit(operation)?);
    let db = Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch("CREATE TRIGGER fail_rename BEFORE UPDATE OF name ON mailboxes WHEN OLD.name='Projects/Review' BEGIN SELECT RAISE(ABORT,'synthetic name write failure'); END;")?;
    assert!(
        archive
            .finish_folder_edit(&frozen, "done", "acknowledged")
            .is_err()
    );
    assert_eq!(archive.folder_operations()?[0].state, "running");
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.id == parent && folder.name == "Projects")
    );
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.id == child && folder.name == "Projects/Review")
    );
    archive.recover_interrupted_folders()?;
    assert_eq!(archive.folder_operations()?[0].state, "unknown");
    assert!(archive.pending_folder_edit("a")?.is_none());
    assert!(archive.cancel_folder_edit(operation).is_err());
    archive.dismiss_folder_edit(operation)?;
    db.execute_batch("DROP TRIGGER fail_rename;")?;
    let second = archive.queue_folder_edit(
        "a",
        FolderEdit::Rename {
            id: parent,
            name: "Work".into(),
        },
    )?;
    let frozen = archive.pending_folder_edit("a")?.ok_or("missing rename")?;
    assert!(archive.claim_folder_edit(second)?);
    archive.finish_folder_edit(&frozen, "done", "acknowledged")?;
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.id == child && folder.name == "Work/Review" && folder.epoch == 8)
    );
    assert_eq!(archive.page(&MailQuery::default(), 10)?.rows[0].id, id);
    assert!(
        archive
            .finish_folder_edit(&frozen, "done", "duplicate")
            .is_err()
    );
    let create = archive.queue_folder_edit("a", FolderEdit::Create("Later".into()))?;
    archive.cancel_folder_edit(create)?;
    assert!(!archive.claim_folder_edit(create)?);
    Ok(())
}

#[test]
fn version_fourteen_upgrade_keeps_folder_identity_and_existing_metadata() -> Result {
    let dir = tempfile::tempdir()?;
    let archive = Archive::open(dir.path())?;
    let id = archive.mailbox("a", "INBOX")?;
    drop(archive);
    let db = Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(concat!(
        include_str!("fixtures/remove_sent_copy.sql"),
        "DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules;"
    ))?;
    db.execute_batch("DROP TABLE occurrence_keywords; DROP TABLE keyword_labels; DROP TABLE keyword_operations; ALTER TABLE move_operations DROP COLUMN kind; DROP TABLE folder_operations; PRAGMA user_version=14;")?;
    let mut archive = Archive::open(dir.path())?;
    assert_eq!(archive.mailboxes()?[0].id, id);
    archive.queue_folder_edit("a", FolderEdit::Create("New".into()))?;
    assert_eq!(archive.folder_operations()?.len(), 1);
    Ok(())
}
