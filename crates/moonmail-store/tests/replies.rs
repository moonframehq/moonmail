use {
    moonmail_core::{BlobStore, MessageId},
    moonmail_mime::StalwartParser,
    moonmail_store::{Archive, FileBlobs},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct MissingBlobs;
impl BlobStore for MissingBlobs {
    type Error = std::io::Error;
    fn put(&self, _: &[u8]) -> std::io::Result<String> {
        unreachable!()
    }
    fn get(&self, _: &str) -> std::io::Result<Vec<u8>> {
        Err(std::io::Error::other("synthetic unavailable blob"))
    }
}

#[test]
fn legacy_headers_recover_lazily_and_migration_preserves_existing_drafts() -> Result {
    let directory = tempfile::tempdir()?;
    let blobs = FileBlobs::new(directory.path());
    let raw = b"From: sender@example.org\r\nReply-To: Team <team@example.org>\r\nTo: me@example.org\r\nCc: peer@example.org\r\nMessage-ID: <parent@example.org>\r\nIn-Reply-To: <root@example.org>\r\n\r\nSynthetic";
    let blob = blobs.put(raw)?;
    let connection = rusqlite::Connection::open(directory.path().join("archive.sqlite3"))?;
    connection.execute_batch(include_str!("../src/schema.sql"))?;
    connection.execute_batch(
        "ALTER TABLE mailboxes ADD COLUMN delimiter TEXT DEFAULT '/';
         ALTER TABLE mailboxes ADD COLUMN selectable INTEGER NOT NULL DEFAULT 1;
         PRAGMA user_version=3;
         INSERT INTO drafts VALUES ('old','recipient@example.org','Untouched','Draft body','parent@example.org',1);",
    )?;
    connection.execute(
        "INSERT INTO messages(id,account,origin,blob,subject,sender,address,recipients,date,snippet,rfc_id,thread_key)
         VALUES (1,'me@example.org','legacy',?1,'Subject','Sender','sender@example.org','me@example.org',1,'','parent@example.org','root@example.org')",
        [&blob],
    )?;
    drop(connection);
    let archive = Archive::open(directory.path())?;
    let draft = archive.latest_draft()?.ok_or("missing draft")?;
    assert_eq!(draft.subject, "Untouched");
    assert_eq!(draft.body, "Draft body");
    assert_eq!(draft.reply_id, "parent@example.org");
    assert!(draft.cc.is_empty() && draft.bcc.is_empty() && draft.account.is_empty());
    assert!(draft.attachments.is_empty());
    assert!(
        draft.references.is_empty(),
        "legacy draft has no invented history"
    );
    assert!(
        archive
            .reply_headers(MessageId(1), &StalwartParser::default(), &MissingBlobs)
            .is_err()
    );
    let headers = archive.reply_headers(MessageId(1), &StalwartParser::default(), &blobs)?;
    assert_eq!(headers.message_id.as_deref(), Some("parent@example.org"));
    assert!(headers.references.is_empty());
    assert_eq!(headers.in_reply_to.as_deref(), Some("root@example.org"));
    assert_eq!(headers.addresses.reply_to[0].address, "team@example.org");
    assert_eq!(headers.addresses.cc[0].address, "peer@example.org");
    drop(archive);
    let archive = Archive::open(directory.path())?;
    assert_eq!(
        archive.reply_headers(MessageId(1), &StalwartParser::default(), &MissingBlobs)?,
        headers,
        "recovered headers persist; no raw reread on subsequent replies"
    );
    assert_eq!(
        blobs.get(&blob)?,
        raw,
        "migration never rewrites original MIME"
    );
    Ok(())
}
