use {
    moonmail_core::BlobStore,
    moonmail_network::{Submission, SubmissionFailure, transmit_queued},
    moonmail_store::{Archive, Draft, FileBlobs},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

#[test]
fn disconnected_outbox_account_is_rejected_before_credentials_or_submission() -> Result {
    let dir = tempfile::tempdir()?;
    moonmail_network::Account::fastmail("another@example.org")?.save(dir.path())?;
    let mut archive = Archive::open(dir.path())?;
    let id = queue(&mut archive, &FileBlobs::new(dir.path()), "disconnected")?;
    let error = moonmail_network::send_queued(dir.path(), id)
        .err()
        .ok_or("unexpected submission")?;
    assert!(matches!(
        error,
        moonmail_network::Error::Invalid("Reconnect the message's sending account")
    ));
    assert_eq!(archive.outbox_item(id)?.state, "queued");
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("synthetic submission failure")]
struct Failure(bool);
struct Fake {
    failure: Option<bool>,
    calls: Vec<(String, Vec<String>, Vec<u8>)>,
}
impl Submission for Fake {
    type Error = Failure;
    fn submit(
        &mut self,
        sender: &str,
        recipients: &[String],
        raw: &[u8],
    ) -> std::result::Result<(), Failure> {
        self.calls
            .push((sender.into(), recipients.to_vec(), raw.to_vec()));
        match self.failure {
            Some(safe) => Err(Failure(safe)),
            None => Ok(()),
        }
    }
    fn failure(error: &Failure) -> SubmissionFailure {
        if error.0 {
            SubmissionFailure::NotSubmitted
        } else {
            SubmissionFailure::Unknown
        }
    }
}

fn queue(archive: &mut Archive, blobs: &FileBlobs, id: &str) -> Result<i64> {
    let draft = Draft {
        id: id.into(),
        account: "me@example.org".into(),
        body: "Frozen".into(),
        ..Default::default()
    };
    Ok(archive.queue(
        &draft,
        &draft.account,
        &blobs.put(b"From: me@example.org\r\n\r\nFrozen")?,
        &["peer@example.org".into(), "hidden@example.org".into()],
    )?)
}

#[test]
fn explicit_retry_of_pre_submission_failure_reuses_identical_mime_and_envelope() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let id = queue(&mut archive, &blobs, "first")?;
    // Exact lookup works even after this item leaves the newest 100 records.
    for n in 0..101 {
        queue(&mut archive, &blobs, &format!("later-{n}"))?;
    }
    let mut fake = Fake {
        failure: Some(true),
        calls: Vec::new(),
    };
    assert!(transmit_queued(&archive, "other@example.org", id, &blobs, &mut fake).is_err());
    assert!(fake.calls.is_empty());
    assert!(transmit_queued(&archive, "me@example.org", id, &blobs, &mut fake).is_err());
    assert_eq!(archive.outbox_item(id)?.state, "failed");
    assert!(transmit_queued(&archive, "me@example.org", id, &blobs, &mut fake).is_err());
    assert_eq!(fake.calls.len(), 1);
    drop(archive);
    let archive = Archive::open(dir.path())?;
    archive.retry_outbox(id)?;
    fake.failure = None;
    transmit_queued(&archive, "me@example.org", id, &blobs, &mut fake)?;
    assert_eq!(fake.calls[0], fake.calls[1]);
    assert_eq!(archive.outbox_item(id)?.state, "sent");
    assert!(archive.retry_outbox(id).is_err());
    Ok(())
}

struct CancelWhileReading<'a> {
    archive: &'a Archive,
    blobs: &'a FileBlobs,
    id: i64,
}
impl BlobStore for CancelWhileReading<'_> {
    type Error = std::io::Error;
    fn put(&self, _: &[u8]) -> std::io::Result<String> {
        unreachable!()
    }
    fn get(&self, hash: &str) -> std::io::Result<Vec<u8>> {
        self.archive
            .cancel_outbox(self.id)
            .map_err(std::io::Error::other)?;
        self.blobs.get(hash).map_err(std::io::Error::other)
    }
}

#[test]
fn lost_acknowledgment_never_retries_and_cancel_winning_the_claim_prevents_submission() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let id = queue(&mut archive, &blobs, "uncertain")?;
    let mut fake = Fake {
        failure: Some(false),
        calls: Vec::new(),
    };
    assert!(transmit_queued(&archive, "me@example.org", id, &blobs, &mut fake).is_err());
    assert_eq!(archive.outbox_item(id)?.state, "unknown");
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    assert!(archive.retry_outbox(id).is_err());
    assert!(archive.restore_outbox_draft(id, "replacement").is_err());
    assert!(transmit_queued(&archive, "me@example.org", id, &blobs, &mut fake).is_err());
    assert_eq!(fake.calls.len(), 1);
    let cancelled = queue(&mut archive, &blobs, "cancelled")?;
    let racing = CancelWhileReading {
        archive: &archive,
        blobs: &blobs,
        id: cancelled,
    };
    assert!(transmit_queued(&archive, "me@example.org", cancelled, &racing, &mut fake).is_err());
    assert_eq!(archive.outbox_item(cancelled)?.state, "cancelled");
    assert_eq!(fake.calls.len(), 1);
    Ok(())
}
