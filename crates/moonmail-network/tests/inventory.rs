use {
    moonmail_core::{MailQuery, MailRepository},
    moonmail_network::{
        Account, IncomingMail, RemoteFolder, RemoteMessage, RemoteSnapshot, ScanError, synchronize,
    },
    moonmail_store::{Archive, FileBlobs},
    std::io,
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Streamed {
    fail: bool,
    epoch: u32,
    count: u32,
    fetched: Vec<u32>,
}
impl IncomingMail for Streamed {
    type Error = io::Error;
    fn folders(&mut self) -> io::Result<Vec<RemoteFolder>> {
        Ok(vec![RemoteFolder {
            name: "INBOX".into(),
            delimiter: None,
            selectable: true,
            role: None,
        }])
    }
    fn snapshot(&mut self, _: &str) -> io::Result<RemoteSnapshot> {
        Err(io::Error::other("unbounded snapshot must not be called"))
    }
    fn scan<S>(
        &mut self,
        _: &str,
        mut sink: impl FnMut(u32, u32, &[RemoteMessage]) -> std::result::Result<(), S>,
    ) -> std::result::Result<(), ScanError<Self::Error, S>> {
        sink(self.epoch, self.count, &[]).map_err(ScanError::Sink)?;
        for start in (1..=self.count).step_by(67) {
            let batch: Vec<_> = (start..=(start + 66).min(self.count))
                .map(|uid| RemoteMessage {
                    keywords: Vec::new(),
                    uid,
                    seen: true,
                    starred: false,
                    size: 120,
                    received_at: 1_700_000_000 + i64::from(uid),
                })
                .collect();
            sink(self.epoch, self.count, &batch).map_err(ScanError::Sink)?;
            if self.fail {
                return Err(ScanError::Incoming(io::Error::other("lost final response")));
            }
        }
        Ok(())
    }
    fn fetch(&mut self, uid: u32) -> io::Result<Option<Vec<u8>>> {
        self.fetched.push(uid);
        Ok(Some(
            format!("From: peer@example.org\r\nSubject: {uid}\r\n\r\nSynthetic").into_bytes(),
        ))
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn streamed_inventory_drives_bounded_missing_pages_and_failed_epoch_scan_cannot_retire_mail()
-> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.org")?;
    let mut server = Streamed {
        fail: false,
        epoch: 7,
        count: 205,
        fetched: Vec::new(),
    };
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.fetched, (1..=205).rev().collect::<Vec<_>>());
    assert_eq!(archive.total()?, 205);
    server.fetched.clear();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert!(server.fetched.is_empty());
    server.epoch = 8;
    server.count = 100;
    server.fail = true;
    assert!(synchronize(&mut server, &account, &mut archive, &blobs, |_| {}).is_err());
    assert_eq!(archive.page(&MailQuery::default(), 200)?.rows.len(), 200);
    assert!(server.fetched.is_empty());
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    server.fail = false;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert_eq!(server.fetched, (1..=100).rev().collect::<Vec<_>>());
    assert_eq!(archive.page(&MailQuery::default(), 200)?.rows.len(), 100);
    assert_eq!(archive.total()?, 305);
    Ok(())
}
