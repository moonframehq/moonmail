//! Opt-in Fastmail interoperability checks for IMAP/SMTP authentication and JMAP delivery.
use {
    moonmail_core::{BlobStore, MailboxRole, MimeParser},
    moonmail_mime::StalwartParser,
    moonmail_network::{Account, SecretString, jmap::Client, prepare_message, transmit_queued},
    moonmail_store::{Archive, Draft, FileBlobs},
    rusqlite::{OptionalExtension, params},
    std::time::{Duration, Instant},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
fn peer(email: &str, token: &str) -> Result<(Account, Client)> {
    let account = Account::fastmail(&std::env::var(email)?)?;
    let token =
        SecretString::from(std::env::var(token).map_err(|_| "Missing Fastmail test token")?);
    let client = Client::fastmail(token).map_err(|_| "Fastmail session discovery failed")?;
    Ok((account, client))
}
#[test]
#[ignore = "sends mail; requires MOONMAIL_LIVE_FASTMAIL=1 and two dedicated Fastmail accounts"]
fn fastmail_round_trip() -> Result {
    if std::env::var("MOONMAIL_LIVE_FASTMAIL").as_deref() != Ok("1") {
        return Err("Set MOONMAIL_LIVE_FASTMAIL=1".into());
    }
    let (sender, mut a) = peer("FASTMAIL_TEST_A_EMAIL", "FASTMAIL_TEST_A_TOKEN")?;
    let (recipient, b) = peer("FASTMAIL_TEST_B_EMAIL", "FASTMAIL_TEST_B_TOKEN")?;
    if a.account_id() == b.account_id() || sender.email.eq_ignore_ascii_case(&recipient.email) {
        return Err("Two distinct primary accounts required".into());
    }
    a.check_identity(&sender.email)?;
    b.check_identity(&recipient.email)?;
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let run = uuid::Uuid::new_v4().to_string();
    let draft = Draft {
        id: run.clone(),
        account: sender.email.clone(),
        recipients: recipient.email.clone(),
        subject: format!("Moonmail interop {run}"),
        body: format!("Synthetic JMAP delivery {run}"),
        ..Default::default()
    };
    let (raw, envelope) = prepare_message(&sender, &draft)?;
    let rfc = StalwartParser::default()
        .parse(&raw)?
        .message_id
        .ok_or("Missing frozen Message-ID")?;
    let outbox = archive.queue_with_message_id(
        &draft,
        &sender.email,
        &blobs.put(&raw)?,
        &envelope,
        Some(&rfc),
    )?;
    archive.prepare_sent_copy(outbox, false)?;
    transmit_queued(&archive, &sender.email, outbox, &blobs, &mut a)
        .map_err(|_| "Submission failed or uncertain; inspect test accounts")?;
    let end = Instant::now() + Duration::from_secs(300);
    for (client, account, role) in [
        (&a, &sender, MailboxRole::Sent),
        (&b, &recipient, MailboxRole::Inbox),
    ] {
        loop {
            client
                .sync(
                    &account.email,
                    &mut archive,
                    &blobs,
                    |_| {},
                    || Instant::now() >= end,
                )
                .map_err(|_| "Live sync failed or timed out")?;
            let folder = archive.role_mailbox(&account.email, role)?;
            let db = rusqlite::Connection::open_with_flags(
                dir.path().join("archive.sqlite3"),
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let blob:Option<String>=db.query_row("SELECT blob FROM messages JOIN occurrences ON message_id=messages.id WHERE mailbox_id=?1 AND rfc_id=?2",params![folder,rfc],|r|r.get(0)).optional()?;
            if let Some(blob) = blob {
                let message = StalwartParser::default().parse(&blobs.get(&blob)?)?;
                if message.subject.as_deref() != Some(&draft.subject)
                    || !message
                        .text_body
                        .as_deref()
                        .is_some_and(|s| s.contains(&draft.body))
                {
                    return Err("Synthetic message content mismatch".into());
                }
                break;
            }
            if Instant::now() >= end {
                return Err("Delivery observation timed out".into());
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    Ok(())
}

/// Exercise the production TLS and SASL adapters without an account or a message.
#[test]
#[ignore = "contacts Fastmail with an intentionally invalid synthetic credential"]
fn fastmail_parses_imap_and_smtp_bearer_commands() -> Result {
    use moonmail_network::{Error, LoginSecret, RustImap, SmtpSubmission, SubmitError};
    let account = Account::fastmail("moonmail-fixture@example.invalid")?;
    let secret = LoginSecret {
        bearer: true,
        value: "synthetic-invalid-token".into(),
    };
    assert!(matches!(
        RustImap::connect(&account, &secret),
        Err(Error::LoginRejected)
    ));
    let result = SmtpSubmission { account, secret }.check_connection();
    assert!(
        matches!(result, Err(SubmitError::Connect(mail_send::Error::AuthenticationFailed(reply))) if reply.code() == 535)
    );
    Ok(())
}
