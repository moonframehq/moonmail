use {
    moonmail_store::{Archive, Draft},
    std::collections::HashSet,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn draft_pages_cover_all_metadata_and_reopen_preserves_independent_content() -> Result {
    let dir = tempfile::tempdir()?;
    let archive = Archive::open(dir.path())?;
    for number in 0..123 {
        archive.save_draft(&Draft {
            id: format!("draft-{number:03}"),
            recipients: "Alice <alice@example.org>".into(),
            cc: "copy@example.org".into(),
            bcc: "private@example.org".into(),
            account: "me@example.org".into(),
            body: format!("Independent body {number}"),
            subject: format!("Subject {number}"),
            ..Default::default()
        })?;
    }
    assert_eq!(archive.draft_count()?, 123);
    let mut cursor = None;
    let mut ids = HashSet::new();
    loop {
        let page = archive.drafts(cursor.as_ref(), 17)?;
        assert!(page.rows.len() <= 17);
        for row in &page.rows {
            assert!(ids.insert(row.id.clone()));
        }
        assert!(
            !serde_json::to_string(&page)?.contains("Independent body"),
            "draft navigation must not load bodies"
        );
        assert!(!serde_json::to_string(&page)?.contains("private@example.org"));
        cursor = page.next;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 123);
    let mut older = archive.draft("draft-000")?.ok_or("missing draft")?;
    older.body = "Edited only this draft".into();
    archive.save_draft(&older)?;
    assert_eq!(
        archive.latest_draft()?.ok_or("missing latest")?.id,
        "draft-000"
    );
    archive.discard_draft("draft-001")?;
    drop(archive);
    let archive = Archive::open(dir.path())?;
    assert_eq!(archive.draft_count()?, 122);
    assert!(archive.draft("draft-001")?.is_none());
    assert_eq!(
        archive.draft("draft-000")?.ok_or("missing edited")?.body,
        "Edited only this draft"
    );
    let other = archive.draft("draft-002")?.ok_or("missing other")?;
    assert_eq!(other.body, "Independent body 2");
    assert_eq!(other.bcc, "private@example.org");
    assert_eq!(other.cc, "copy@example.org");
    assert_eq!(other.account, "me@example.org");
    Ok(())
}

#[test]
fn migration_and_lost_append_receipt_preserve_newer_edits_and_discard_intent() -> Result {
    let dir = tempfile::tempdir()?;
    let mut draft = Draft {
        id: "migrate".into(),
        account: "me@example.org".into(),
        body: "before".into(),
        ..Default::default()
    };
    Archive::open(dir.path())?.save_draft(&draft)?;
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(include_str!("fixtures/remove_remote_drafts.sql"))?;
    db.execute_batch("PRAGMA user_version=20")?;
    let mut archive = Archive::open(dir.path())?;
    let (_, revision) = archive
        .dirty_draft(&draft.account, 0)?
        .ok_or("legacy draft was lost")?;
    assert!(archive.claim_draft_upload(&draft, revision, "Drafts", "first@moonmail.local")?);
    draft.body = "during APPEND".into();
    archive.save_draft(&draft)?;
    assert!(archive.accept_draft_upload(&draft.account, "Drafts", 7, 3, "first@moonmail.local")?);
    let (_, next) = archive
        .dirty_draft(&draft.account, 0)?
        .ok_or("concurrent edit lost")?;
    assert!(next > revision);
    assert!(archive.claim_draft_upload(&draft, next, "Drafts", "next@moonmail.local")?);
    archive.discard_draft(&draft.id)?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert!(archive.draft(&draft.id)?.is_none());
    assert!(archive.accept_draft_upload(&draft.account, "Drafts", 7, 4, "next@moonmail.local")?);
    let ops = archive.remote_operations(Some(&draft.account))?;
    assert_eq!(
        ops.iter().map(|op| (op.epoch, op.uid)).collect::<Vec<_>>(),
        [(7, 3), (7, 4)]
    );
    assert!(archive.claim_expunge(ops[0].id)?);
    assert!(!archive.claim_expunge(ops[0].id)?);
    assert_eq!(
        Archive::open(dir.path())?
            .remote_operations(Some(&draft.account))?
            .len(),
        1
    );
    Ok(())
}
