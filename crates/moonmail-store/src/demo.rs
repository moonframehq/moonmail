use {
    crate::{Archive, BlobError, FileBlobs, IncomingMessage, IngestError},
    moonmail_mime::{StalwartError, StalwartParser},
};

pub fn seed_demo(
    archive: &mut Archive,
    blobs: &FileBlobs,
) -> Result<(), IngestError<StalwartError, BlobError>> {
    for name in [
        "Projects",
        "Projects/Moonmail",
        "Projects/Moonmail/Design",
        "Personal",
        "Personal/Reading",
    ] {
        archive.mailbox("you@example.com", name)?;
    }
    if archive.total()? > 0 {
        return Ok(());
    }
    let parser = StalwartParser::default();
    let account = "you@example.com";
    let people = [
        ("Clara Jensen", "clara@example.org"),
        ("Julian Park", "julian@example.org"),
        ("Maya Laurent", "maya@example.net"),
        ("Theo Martin", "theo@example.com"),
        ("Nora Reed", "nora@example.org"),
    ];
    let subjects = [
        "A quieter place to work",
        "Notes from the coast",
        "The November reading list",
        "Studio visit next Thursday",
        "A small idea for the weekend",
        "Photographs from Lisbon",
        "The new identity is ready",
        "Dinner at seven?",
        "One more thought on the proposal",
    ];
    for number in 0..72 {
        let group = number / 3;
        let (name, address) = people[group as usize % people.len()];
        let outbound = number % 3 == 1;
        let folder = if outbound {
            "Sent"
        } else if group % 4 == 3 {
            "Archive"
        } else {
            "INBOX"
        };
        let mailbox = archive.mailbox(account, folder)?;
        let subject = subjects[group as usize % subjects.len()];
        let sender = if outbound {
            format!("You <{account}>")
        } else {
            format!("{name} <{address}>")
        };
        let recipient = if outbound { address } else { account };
        let reference = if number % 3 == 0 {
            String::new()
        } else {
            format!(
                "References: <demo-{}@moonmail.local>\r\nIn-Reply-To: <demo-{}@moonmail.local>\r\n",
                group * 3,
                group * 3
            )
        };
        let date = 1_793_707_200 - number * 3600;
        let raw = format!(
            "From: {sender}\r\nTo: {recipient}\r\nSubject: {}{subject}\r\nMessage-ID: <demo-{number}@moonmail.local>\r\n{reference}MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHi {},\r\n\r\n{}\r\n\r\nI keep coming back to the idea that the best tools give us room to think. A little less noise, a little more attention to the things that matter.\r\n\r\nWould you have a moment to take a look this week? No rush — I'd love to hear your thoughts when you have time.\r\n\r\nWarmly,\r\n{}\r\n\r\n—\r\nThis is a synthetic demo message. No account is connected.\r\n",
            if number % 3 > 0 { "Re: " } else { "" },
            if outbound { name } else { "you" },
            if outbound {
                "Thanks for sending this over. I read through it this morning and have a few ideas to share."
            } else {
                "I wanted to send you a few notes before the week gets away from us."
            },
            if outbound { "You" } else { name }
        );
        archive.ingest(
            &parser,
            blobs,
            IncomingMessage {
                account,
                mailbox,
                origin: &format!("demo:{number}"),
                epoch: 0,
                uid: None,
                seen: number % 4 != 0,
                starred: number % 13 == 0,
                received_at: date,
                raw: raw.as_bytes(),
            },
        )?;
    }
    // These empty folders make the demo's sidebar useful without fake counts.
    archive
        .mailbox(account, "Junk")
        .map_err(IngestError::Store)?;
    archive
        .mailbox(account, "Trash")
        .map_err(IngestError::Store)?;
    Ok(())
}
