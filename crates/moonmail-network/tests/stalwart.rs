//! Run with `just test-stalwart`. All server and archive data are disposable.
use {
    moonmail_core::{BlobStore, MailRepository, MailboxRole, MessageId, MimeParser},
    moonmail_mime::StalwartParser,
    moonmail_network::{Account, SecretString, jmap::Client, prepare_message, transmit_queued},
    moonmail_store::{Archive, Draft, FileBlobs, FolderEdit, MoveTarget},
    rusqlite::{Connection, OpenFlags, params},
    serde_json::Value,
    std::{
        io::{BufRead, BufReader, Read},
        path::Path,
        time::{Duration, Instant},
    },
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const ALICE: &str = "alice@moonmail.example.org";
const BOB: &str = "bob@moonmail.example.org";

fn peer(session: &str, key: &str, email: &str) -> Result<Client> {
    let token = SecretString::from(std::env::var(key).map_err(|_| "Missing fixture token")?);
    let client = Client::discover(session, token)?;
    client.check_identity(email)?;
    Ok(client)
}

fn sync(client: &Client, address: &str, archive: &mut Archive, blobs: &FileBlobs) -> Result {
    let deadline = Instant::now() + Duration::from_secs(30);
    client.sync(
        address,
        archive,
        blobs,
        |_| {},
        || Instant::now() >= deadline,
    )?;
    Ok(())
}

fn verify_event(stream: impl Read, account: &str) -> Result {
    let mut event = String::new();
    let mut data = String::new();
    for line in BufReader::new(stream.take(65536)).lines() {
        let line = line?;
        if line.is_empty() {
            if event == "state" {
                let state: Value = serde_json::from_str(&data)?;
                assert_eq!(state["@type"], "StateChange");
                assert!(
                    state["changed"][account]["Email"].is_string(),
                    "Push omitted recipient email state"
                );
                return Ok(());
            }
            event.clear();
            data.clear();
        } else if let Some(kind) = line.strip_prefix("event:") {
            event = kind.trim_start().to_owned();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push_str(value.trim_start());
            data.push('\n');
        }
    }
    Err("No recipient Email state event received".into())
}

fn delivered(
    archive: &Archive,
    root: &Path,
    account: &str,
    role: MailboxRole,
    rfc_id: &str,
    body: &str,
) -> Result<(MessageId, String)> {
    let folder = archive.role_mailbox(account, role)?;
    let db = Connection::open_with_flags(
        root.join("archive.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let (id, remote, count): (i64, String, i64) = db.query_row(
        "SELECT m.id,e.remote,COUNT(*) FROM messages m JOIN occurrences o ON m.id=o.message_id
         JOIN jmap_emails e ON e.uid=o.uid AND e.account=m.account
         WHERE m.account=?1 AND o.mailbox_id=?2 AND m.rfc_id=?3 HAVING COUNT(*)>0",
        params![account, folder, rfc_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    assert_eq!(count, 1);
    let id = MessageId(id as u64);
    let bytes = FileBlobs::new(root).get(&archive.message_blob(id)?)?;
    let message = StalwartParser::default().parse(&bytes)?;
    assert_eq!(message.message_id.as_deref(), Some(rfc_id));
    assert_eq!(message.text_body.as_deref().map(str::trim), Some(body));
    Ok((id, remote))
}

fn membership(root: &Path, id: MessageId) -> Result<Vec<String>> {
    let db = Connection::open_with_flags(
        root.join("archive.sqlite3"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    let mut query=db.prepare("SELECT b.name FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id WHERE message_id=?1 ORDER BY b.name")?;
    Ok(query
        .query_map([id.0 as i64], |r| r.get(0))?
        .collect::<rusqlite::Result<_>>()?)
}

#[test]
#[ignore = "requires the two disposable accounts created by scripts/test_stalwart.py"]
fn two_account_round_trip() -> Result {
    let session = std::env::var("MOONMAIL_STALWART_URL")?;
    let url = url::Url::parse(&session)?;
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    let mut a = peer(&session, "MOONMAIL_STALWART_ALICE_TOKEN", ALICE)?;
    let b = peer(&session, "MOONMAIL_STALWART_BOB_TOKEN", BOB)?;
    assert_ne!(a.account_id(), b.account_id());
    let dir = tempfile::tempdir()?;
    let root = dir.path();
    let mut archive = Archive::open(root)?;
    let blobs = FileBlobs::new(root);
    sync(&b, BOB, &mut archive, &blobs)?;
    let before = archive.jmap_states(BOB)?.1;
    let events = b.events()?;
    let run = uuid::Uuid::new_v4().to_string();
    let draft = Draft {
        id: run.clone(),
        account: ALICE.into(),
        recipients: BOB.into(),
        subject: format!("Stalwart {run}"),
        body: format!("Moonmail synthetic round trip {run}"),
        ..Default::default()
    };
    let (raw, envelope) = prepare_message(&Account::fastmail(ALICE)?, &draft)?;
    let rfc = StalwartParser::default()
        .parse(&raw)?
        .message_id
        .ok_or("Missing frozen Message-ID")?;
    let outbox =
        archive.queue_with_message_id(&draft, ALICE, &blobs.put(&raw)?, &envelope, Some(&rfc))?;
    archive.prepare_sent_copy(outbox, false)?;
    transmit_queued(&archive, ALICE, outbox, &blobs, &mut a)?;
    assert_eq!(archive.outbox_item(outbox)?.state, "sent");
    verify_event(events, b.account_id())?;
    println!("Recipient push verified after frozen outbox submission");
    drop(archive);
    let mut archive = Archive::open(root)?;
    sync(&b, BOB, &mut archive, &blobs)?;
    assert_ne!(archive.jmap_states(BOB)?.1, before);
    let (id, remote) = delivered(&archive, root, BOB, MailboxRole::Inbox, &rfc, &draft.body)?;
    sync(&a, ALICE, &mut archive, &blobs)?;
    let (sent, _) = delivered(&archive, root, ALICE, MailboxRole::Sent, &rfc, &draft.body)?;
    assert_eq!(
        archive.message_blob(sent)?,
        archive.outbox_item(outbox)?.blob,
        "Sent MIME must match the frozen outbox bytes"
    );
    println!("Verified exact Message-ID and synthetic body in Inbox and Sent");

    archive.set_flag(id, "seen", true)?;
    archive.set_flag(id, "flagged", true)?;
    archive.set_keywords(&[id], "project", true)?;
    sync(&b, BOB, &mut archive, &blobs)?;
    // A fresh archive has no optimistic local edits. It observes the server independently.
    let observer = tempfile::tempdir()?;
    let mut witness = Archive::open(observer.path())?;
    let observed_blobs = FileBlobs::new(observer.path());
    sync(&b, BOB, &mut witness, &observed_blobs)?;
    let observed = witness
        .jmap_message(BOB, &remote)?
        .ok_or("Missing observed email")?;
    let flags = witness.detail(observed)?.summary;
    assert!(!flags.unread && flags.starred);
    assert!(
        witness
            .message_keywords(observed)?
            .iter()
            .any(|k| k == "project")
    );
    archive.set_flag(id, "seen", false)?;
    archive.set_flag(id, "flagged", false)?;
    archive.set_keywords(&[id], "project", false)?;
    sync(&b, BOB, &mut archive, &blobs)?;
    sync(&b, BOB, &mut witness, &observed_blobs)?;
    let flags = witness.detail(observed)?.summary;
    assert!(flags.unread && !flags.starred);
    assert!(
        !witness
            .message_keywords(observed)?
            .iter()
            .any(|k| k == "project")
    );

    archive.queue_folder_edit(BOB, FolderEdit::Create("Filed".into()))?;
    sync(&b, BOB, &mut archive, &blobs)?;
    let (_, folder, _) = archive
        .jmap_folder_map(BOB)?
        .into_iter()
        .find(|(_, _, name)| name == "Filed")
        .ok_or("Missing created folder")?;
    archive.queue_folder_edit(
        BOB,
        FolderEdit::Rename {
            id: folder,
            name: "Reviewed".into(),
        },
    )?;
    sync(&b, BOB, &mut archive, &blobs)?;
    let inbox = archive.role_mailbox(BOB, MailboxRole::Inbox)?;
    archive.queue_copies(&[id], Some(inbox), MoveTarget::Folder(folder))?;
    sync(&b, BOB, &mut archive, &blobs)?;
    sync(&b, BOB, &mut witness, &observed_blobs)?;
    assert_eq!(
        membership(observer.path(), observed)?,
        ["Inbox", "Reviewed"]
    );
    archive.queue_moves(&[id], Some(inbox), MoveTarget::Role(MailboxRole::Archive))?;
    sync(&b, BOB, &mut archive, &blobs)?;
    sync(&b, BOB, &mut witness, &observed_blobs)?;
    assert_eq!(
        membership(observer.path(), observed)?,
        ["Archived mail", "Reviewed"]
    );
    archive.queue_moves(&[id], Some(folder), MoveTarget::Role(MailboxRole::Trash))?;
    sync(&b, BOB, &mut archive, &blobs)?;
    archive.queue_folder_edit(BOB, FolderEdit::Delete(folder))?;
    sync(&b, BOB, &mut archive, &blobs)?;
    sync(&b, BOB, &mut witness, &observed_blobs)?;
    delivered(
        &witness,
        observer.path(),
        BOB,
        MailboxRole::Trash,
        &rfc,
        &draft.body,
    )?;
    assert!(
        !witness
            .jmap_folder_map(BOB)?
            .iter()
            .any(|(_, _, n)| n == "Reviewed")
    );
    b.destroy_email(&remote)?;
    sync(&b, BOB, &mut witness, &observed_blobs)?;
    assert!(membership(observer.path(), observed)?.is_empty());
    println!(
        "Independent archive verified flags, keywords, folder edits, copy, archive, trash and destroy"
    );
    Ok(())
}
