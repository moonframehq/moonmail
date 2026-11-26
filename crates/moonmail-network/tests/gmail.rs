use {
    moonmail_core::{MailQuery, MailRepository, MailboxRole},
    moonmail_network::{
        Account, IncomingMail, KeywordOutcome, Provider, RemoteFolder, RemoteMessage,
        RemoteSnapshot, synchronize,
    },
    moonmail_store::{Archive, FileBlobs, GmailMetadata, MoveTarget},
    std::io,
};

#[derive(Default)]
struct Gmail {
    scans: Vec<String>,
    downloads: usize,
    edits: Vec<(String, bool)>,
    archived: bool,
}

impl IncomingMail for Gmail {
    type Error = io::Error;
    fn folders(&mut self) -> io::Result<Vec<RemoteFolder>> {
        Ok([
            ("INBOX", MailboxRole::Inbox),
            ("Tous les messages", MailboxRole::Archive),
            ("Importants", MailboxRole::Important),
        ]
        .into_iter()
        .map(|(name, role)| RemoteFolder {
            name: name.into(),
            role: Some(role),
            selectable: true,
            delimiter: Some("/".into()),
        })
        .collect())
    }
    fn snapshot(&mut self, folder: &str) -> io::Result<RemoteSnapshot> {
        self.scans.push(folder.into());
        let mut labels = std::collections::BTreeSet::from(["My Project".to_owned()]);
        for (label, add) in &self.edits {
            if *add {
                labels.insert(label.clone());
            } else {
                labels.remove(label);
            }
        }
        let messages = if folder == "INBOX" && self.archived {
            Vec::new()
        } else {
            vec![RemoteMessage {
                gmail: Some(GmailMetadata {
                    msgid: u64::MAX,
                    thrid: u64::MAX - 1,
                    labels: labels.into_iter().collect(),
                }),
                keywords: vec!["work".into()],
                uid: if folder == "INBOX" { 10 } else { 20 },
                seen: true,
                starred: false,
                size: 120,
                received_at: 1_700_000_000,
            }]
        };
        Ok(RemoteSnapshot { epoch: 7, messages })
    }
    fn fetch(&mut self, _uid: u32) -> io::Result<Option<Vec<u8>>> {
        self.downloads += 1;
        Ok(Some(
            b"From: peer@example.org\r\nSubject: synthetic\r\n\r\nShared".to_vec(),
        ))
    }
    fn gmail_label(
        &mut self,
        folder: &str,
        epoch: u32,
        uid: u32,
        label: &str,
        value: bool,
    ) -> io::Result<KeywordOutcome> {
        assert_eq!((folder, epoch, uid), ("Tous les messages", 7, 20));
        self.edits.push((label.to_owned(), value));
        if label == "\\Inbox" && !value {
            self.archived = true;
        }
        Ok(KeywordOutcome::Applied)
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> io::Result<()> {
        unreachable!()
    }
}

#[test]
fn selective_gmail_inventory_reuses_bytes_and_replays_durable_labels()
-> Result<(), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let mut archive = Archive::open(root.path())?;
    let blobs = FileBlobs::new(root.path());
    let account = Account::preset(Provider::Google, "synthetic@example.org")?;
    let mut server = Gmail::default();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.scans, ["INBOX", "Tous les messages"]);
    assert_eq!(server.downloads, 1);
    assert_eq!(archive.total()?, 1);
    let folders = archive.mailboxes()?;
    let important = folders
        .iter()
        .find(|f| f.role == Some(MailboxRole::Important))
        .ok_or("missing folder")?;
    assert!(!important.sync_enabled);
    archive.set_sync_enabled(important.id, true)?;
    let id = archive.page(&MailQuery::default(), 3)?.rows[0].id;
    assert_eq!(archive.gmail_labels(id)?, Some(vec!["My Project".into()]));
    archive.set_gmail_labels(&[id], "My Project", false)?;
    archive.set_gmail_labels(&[id], "To Review", true)?;
    archive.queue_moves(&[id], None, MoveTarget::Role(MailboxRole::Archive))?;
    drop(archive);
    let mut archive = Archive::open(root.path())?;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(
        server.edits,
        [
            ("My Project".into(), false),
            ("To Review".into(), true),
            ("\\Inbox".into(), false)
        ]
    );
    assert_eq!(server.downloads, 1);
    assert_eq!(archive.gmail_labels(id)?, Some(vec!["To Review".into()]));
    assert_eq!(archive.unresolved_keyword_count()?, 0);
    assert!(server.scans.iter().any(|f| f == "Importants"));
    for folder in archive.mailboxes()? {
        assert_eq!(folder.total, u64::from(folder.name != "INBOX"));
    }
    // Gmail IDs do not identify content across accounts.
    let other = Account::preset(Provider::Google, "other@example.org")?;
    synchronize(&mut server, &other, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.downloads, 2);
    assert_eq!(archive.total()?, 2);
    Ok(())
}

#[test]
fn upgraded_occurrences_converge_without_downloading_or_losing_pending_tags()
-> Result<(), Box<dyn std::error::Error>> {
    use moonmail_store::IncomingMessage;
    let root = tempfile::tempdir()?;
    let mut archive = Archive::open(root.path())?;
    let blobs = FileBlobs::new(root.path());
    let account = Account::preset(Provider::Google, "synthetic@example.org")?;
    for (name, uid) in [("INBOX", 10), ("Tous les messages", 20)] {
        let mailbox = archive.mailbox(&account.email, name)?;
        let id = archive.ingest(
            &moonmail_mime::StalwartParser::default(),
            &blobs,
            IncomingMessage {
                account: &account.email,
                mailbox,
                origin: &format!("imap:{mailbox}:7:{uid}"),
                epoch: 7,
                uid: Some(uid),
                seen: true,
                starred: false,
                received_at: 0,
                raw: b"Subject: old copy\r\n\r\nSynthetic",
            },
        )?;
        if uid == 20 {
            archive.set_keywords(&[id], "pending", true)?;
        }
    }
    let mut server = Gmail::default();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.downloads, 0);
    let rows = archive
        .page(
            &MailQuery {
                threaded: false,
                ..Default::default()
            },
            10,
        )?
        .rows;
    assert_eq!(rows.len(), 1);
    assert!(
        archive
            .message_keywords(rows[0].id)?
            .iter()
            .any(|k| k == "pending")
    );
    Ok(())
}
