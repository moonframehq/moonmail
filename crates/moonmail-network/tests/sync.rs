use {
    moonmail_core::{MailQuery, MailRepository},
    moonmail_network::{
        Account, IncomingMail, RemoteFolder, RemoteMessage, RemoteSnapshot, synchronize,
    },
    moonmail_store::{Archive, FileBlobs},
    std::error::Error,
};

struct Fake {
    fail: bool,
}
impl IncomingMail for Fake {
    type Error = std::io::Error;
    fn folders(&mut self) -> Result<Vec<RemoteFolder>, Self::Error> {
        Ok(vec![
            RemoteFolder {
                name: "Parent".into(),
                delimiter: Some(".".into()),
                selectable: false,
            },
            RemoteFolder {
                delimiter: Some("/".into()),
                selectable: true,
                name: "INBOX".into(),
            },
        ])
    }
    fn snapshot(&mut self, folder: &str) -> Result<RemoteSnapshot, Self::Error> {
        assert_ne!(folder, "Parent", "NOSELECT folders must not be selected");
        Ok(RemoteSnapshot {
            epoch: 7,
            messages: (1..=2)
                .map(|uid| RemoteMessage {
                    uid,
                    seen: false,
                    starred: false,
                    size: 100,
                })
                .collect(),
        })
    }
    fn fetch(&mut self, uid: u32) -> Result<Option<Vec<u8>>, Self::Error> {
        if self.fail && uid == 1 {
            return Err(std::io::Error::other("synthetic disconnect"));
        }
        Ok(Some(format!("From: sender@example.org\r\nTo: me@example.com\r\nSubject: {uid}\r\n\r\nSynthetic body").into_bytes()))
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> Result<(), Self::Error> {
        Ok(())
    }
}

#[test]
fn interrupted_sync_resumes_without_skipping_or_duplicating_messages() -> Result<(), Box<dyn Error>>
{
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.com")?;
    assert!(
        synchronize(
            &mut Fake { fail: true },
            &account,
            &mut archive,
            &blobs,
            |_| {}
        )
        .is_err()
    );
    assert_eq!(archive.total()?, 1);
    synchronize(
        &mut Fake { fail: false },
        &account,
        &mut archive,
        &blobs,
        |_| {},
    )?;
    assert_eq!(archive.total()?, 2);
    synchronize(
        &mut Fake { fail: false },
        &account,
        &mut archive,
        &blobs,
        |_| {},
    )?;
    assert_eq!(archive.total()?, 2);
    assert_eq!(archive.page(&MailQuery::default(), 100)?.rows.len(), 2);
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.name == "Parent"
                && folder.delimiter.as_deref() == Some(".")
                && !folder.selectable)
    );
    Ok(())
}
