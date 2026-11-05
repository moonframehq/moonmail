use {
    moonmail_store::{Archive, Draft, DraftAttachment},
    std::collections::HashSet,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

fn draft(id: &str) -> Draft {
    Draft {
        id: id.into(),
        account: "me@example.org".into(),
        recipients: "Peer <peer@example.org>".into(),
        cc: "copy@example.org".into(),
        bcc: "hidden@example.org".into(),
        subject: "Synthetic recovery".into(),
        body: "Exact saved body".into(),
        reply_id: "parent@example.org".into(),
        references: vec!["root@example.org".into()],
        attachments: vec![DraftAttachment {
            id: "file-one".into(),
            name: "notes.txt".into(),
            blob: "a".repeat(64),
            bytes: 7,
        }],
        identity: None,
    }
}

#[test]
fn restore_is_atomic_preserves_full_snapshot_and_cannot_duplicate_or_resend_original() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let original = draft("original");
    archive.save_draft(&original)?;
    let id = archive.queue(
        &original,
        &original.account,
        "frozen",
        &["peer@example.org".into(), "hidden@example.org".into()],
    )?;
    assert!(archive.outbox_item(id)?.editable);
    let navigation = serde_json::to_string(&archive.outbox()?)?;
    assert!(!navigation.contains("Exact saved body") && !navigation.contains("notes.txt"));
    let connection = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    connection.execute_batch("CREATE TRIGGER reject_restore BEFORE INSERT ON drafts BEGIN SELECT RAISE(ABORT,'synthetic disk failure'); END;")?;
    assert!(archive.restore_outbox_draft(id, "restored").is_err());
    assert_eq!(archive.outbox_item(id)?.state, "queued");
    assert!(archive.draft("restored")?.is_none());
    connection.execute_batch("DROP TRIGGER reject_restore;")?;
    let restored = archive.restore_outbox_draft(id, "restored")?;
    let mut expected = serde_json::to_value(&original)?;
    expected["id"] = "restored".into();
    assert_eq!(serde_json::to_value(&restored)?, expected);
    assert_eq!(archive.outbox_item(id)?.state, "cancelled");
    assert!(!archive.claim_send(id)?);
    assert!(archive.save_draft(&original).is_err());
    assert!(archive.restore_outbox_draft(id, "duplicate").is_err());
    assert_eq!(archive.draft_count()?, 1);
    drop(archive);
    let archive = Archive::open(dir.path())?;
    assert_eq!(serde_json::to_value(archive.draft("restored")?)?, expected);
    assert_eq!(archive.outbox_item(id)?.blob, "frozen");
    Ok(())
}

#[test]
fn only_proven_unsent_states_allow_recovery_and_legacy_snapshots_are_not_invented() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    for state in ["sending", "sent", "unknown", "cancelled", "failed"] {
        let draft = draft(state);
        let id = archive.queue(
            &draft,
            &draft.account,
            "frozen",
            &["peer@example.org".into()],
        )?;
        if state == "cancelled" {
            archive.cancel_outbox(id)?;
        } else {
            assert!(archive.claim_send(id)?);
            if state != "sending" {
                archive.send_result(id, state, "synthetic")?;
            }
        }
        if state == "failed" {
            archive.retry_outbox(id)?;
            assert_eq!(archive.outbox_item(id)?.state, "queued");
            assert!(archive.retry_outbox(id).is_err());
            archive.cancel_outbox(id)?;
        } else {
            assert!(archive.retry_outbox(id).is_err());
            assert!(archive.cancel_outbox(id).is_err());
            assert!(
                archive
                    .restore_outbox_draft(id, &format!("new-{state}"))
                    .is_err()
            );
        }
        assert!(!archive.claim_send(id)?);
    }
    let original = draft("legacy");
    let legacy = archive.queue(
        &original,
        &original.account,
        "frozen",
        &["peer@example.org".into()],
    )?;
    let connection = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    connection.execute("UPDATE outbox SET draft_json=NULL WHERE id=?1", [legacy])?;
    assert!(!archive.outbox_item(legacy)?.editable);
    assert!(
        archive
            .restore_outbox_draft(legacy, "legacy-recovery")
            .is_err()
    );
    assert_eq!(archive.outbox_item(legacy)?.state, "queued");
    Ok(())
}

#[test]
fn all_outbox_records_are_pageable_and_old_ids_remain_addressable() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let mut first = None;
    for n in 0..133 {
        let draft = draft(&n.to_string());
        let id = archive.queue(
            &draft,
            &draft.account,
            "frozen",
            &["peer@example.org".into()],
        )?;
        first.get_or_insert(id);
    }
    let first = first.ok_or("missing first")?;
    assert_eq!(archive.outbox_count()?, 133);
    assert_eq!(archive.outbox()?.len(), 100);
    let mut seen = HashSet::new();
    let mut cursor = None;
    loop {
        let page = archive.outbox_page(cursor, 17)?;
        for item in page.rows {
            assert!(seen.insert(item.id));
        }
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen.len(), 133);
    assert_eq!(archive.outbox_item(first)?.id, first);
    assert!(archive.claim_send(first)?);
    Ok(())
}
