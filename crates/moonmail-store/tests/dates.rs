use {
    moonmail_core::MailRepository,
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs, IncomingMessage},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn header_and_source_dates_survive_restart_and_legacy_migration_does_not_invent_them() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let inbox = archive.mailbox("me@example.org", "INBOX")?;
    let mut ids = Vec::new();
    for (n, header) in [
        "Date: Fri, 21 Nov 1997 09:55:06 -0600\r\n",
        "Date: malformed\r\n",
        "",
    ]
    .into_iter()
    .enumerate()
    {
        let raw =
            format!("{header}From: sender@example.org\r\nSubject: Synthetic dates\r\n\r\nBody");
        ids.push(archive.ingest(
            &StalwartParser::default(),
            &FileBlobs::new(dir.path()),
            IncomingMessage {
                account: "me@example.org",
                mailbox: inbox,
                origin: &n.to_string(),
                epoch: 7,
                uid: Some(n as u32 + 1),
                seen: true,
                starred: false,
                received_at: 1_700_000_000,
                raw: raw.as_bytes(),
            },
        )?);
    }
    drop(archive);
    let archive = Archive::open(dir.path())?;
    for (n, id) in ids.iter().enumerate() {
        let detail = archive.detail(*id)?;
        assert_eq!(detail.received_at, Some(1_700_000_000));
        if n == 0 {
            assert_eq!(detail.header_date, Some(880_127_706));
            assert_eq!(detail.summary.date, 880_127_706);
        } else {
            assert_eq!(detail.header_date, None);
            assert_eq!(detail.summary.date, 1_700_000_000);
        }
    }
    drop(archive);
    let db = rusqlite::Connection::open(dir.path().join("archive.sqlite3"))?;
    db.execute_batch(concat!(
        include_str!("fixtures/remove_sent_copy.sql"),
        "DROP TABLE search_partitions; DROP TABLE keyword_rule_runs; DROP TABLE keyword_rules;"
    ))?;
    db.execute_batch("DROP TABLE occurrence_keywords; DROP TABLE keyword_labels; DROP TABLE keyword_operations; ALTER TABLE move_operations DROP COLUMN kind; DROP TABLE folder_operations; DROP TABLE thread_pending; DROP TABLE thread_edges; DROP TABLE thread_members; DROP TABLE thread_nodes; DROP TABLE search_sources; DROP TABLE saved_searches; ALTER TABLE messages DROP COLUMN header_date; ALTER TABLE messages DROP COLUMN received_at; PRAGMA user_version=10;")?;
    drop(db);
    let archive = Archive::open(dir.path())?;
    let legacy = archive.detail(ids[0])?;
    assert_eq!(legacy.summary.date, 880_127_706);
    assert!(legacy.header_date.is_none() && legacy.received_at.is_none());
    Ok(())
}
