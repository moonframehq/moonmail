use {
    crate::{
        DraftAppend, Error, ExpungeOutcome, IncomingMail, Result, SyncError, sync::check_cancel,
    },
    mailparse::MailHeaderMap,
    moonmail_core::{BlobStore, MimeAttachments, MimeParser},
    moonmail_store::{Archive, Draft, DraftAttachment, FileBlobs},
};

fn encode(draft: &Draft, message_id: &str, attachments: &FileBlobs) -> Result<Vec<u8>> {
    let mut builder = mail_builder::MessageBuilder::new()
        .message_id(message_id)
        .from(mail_builder::headers::address::Address::new_address(
            draft
                .identity
                .as_ref()
                .map(|i| i.name.as_str())
                .filter(|n| !n.is_empty()),
            draft.sender(),
        ))
        .subject(draft.subject.as_str())
        .text_body(draft.body.as_str());
    // A draft may contain incomplete addresses. Keep the editable header text,
    // including Bcc, while rejecting injected header delimiters.
    for (name, value) in [
        ("To", &draft.recipients),
        ("Cc", &draft.cc),
        ("Bcc", &draft.bcc),
    ] {
        if value.chars().any(char::is_control) {
            return Err(Error::Invalid("Invalid draft address header"));
        }
        if !value.is_empty() {
            builder = builder.header(name, mail_builder::headers::raw::Raw::new(value.as_str()));
        }
    }
    if draft.subject.chars().any(char::is_control) {
        return Err(Error::Invalid("Invalid draft subject"));
    }
    if !draft.reply_id.is_empty() {
        builder = builder.in_reply_to(draft.reply_id.as_str());
    }
    let references = crate::submission::reply_references(draft)?;
    if !references.is_empty() {
        builder = builder.references(references);
    }
    if let Some(identity) = &draft.identity
        && !identity.reply_to.is_empty()
    {
        builder = builder.reply_to(identity.reply_to.as_str());
    }
    for item in &draft.attachments {
        let bytes = attachments.get(&item.blob)?;
        builder = builder.attachment("application/octet-stream", item.name.as_str(), bytes);
    }
    Ok(builder.write_to_vec()?)
}

fn decode(
    raw: &[u8],
    account: &str,
    body: String,
    attachments: &FileBlobs,
    identities: &[moonmail_store::Identity],
) -> Result<Draft> {
    let parsed = moonmail_mime::StalwartParser::default().parse(raw)?;
    let (headers, _) = mailparse::parse_headers(raw)?;
    let mut draft = Draft {
        id: uuid::Uuid::new_v4().to_string(),
        account: account.into(),
        recipients: headers.get_all_values("To").join(", "),
        cc: headers.get_all_values("Cc").join(", "),
        bcc: headers.get_all_values("Bcc").join(", "),
        subject: parsed.subject.unwrap_or_default(),
        body,
        reply_id: parsed.in_reply_to.unwrap_or_default(),
        references: parsed.references,
        identity: parsed
            .from
            .first()
            .and_then(|from| {
                identities.iter().find(|alias| {
                    crate::recipients::address_key(&alias.address)
                        == crate::recipients::address_key(&from.address)
                })
            })
            .cloned(),
        ..Draft::default()
    };
    let parser = moonmail_mime::MailparseParser;
    let files = parser.attachments(raw)?;
    if files.len() > moonmail_store::MAX_DRAFT_ATTACHMENTS
        || files.iter().map(|f| f.bytes).sum::<u64>() > moonmail_store::MAX_ATTACHMENT_BYTES
    {
        return Err(Error::Invalid(
            "Remote draft exceeds composer attachment limits",
        ));
    }
    for item in files {
        let bytes = parser.attachment(raw, &item.id)?;
        draft.attachments.push(DraftAttachment {
            id: item.id,
            name: item.name,
            bytes: item.bytes,
            blob: attachments.put(&bytes)?,
        });
    }
    Ok(draft)
}

pub(crate) fn sync<I: IncomingMail>(
    incoming: &mut I,
    account: &str,
    archive: &mut Archive,
    blobs: &FileBlobs,
    (folder, mailbox, epoch): (&str, i64, u32),
    cancelled: &impl Fn() -> bool,
) -> std::result::Result<(), SyncError<I::Error>> {
    let attachments = blobs.draft_attachments();
    let identities = archive.identities(account)?;
    let mut first_error = None;
    archive.retire_missing_drafts(account, folder, mailbox)?;
    let mut after = 0;
    loop {
        check_cancel(cancelled)?;
        let Some(source) = archive.unimported_draft(account, folder, mailbox, epoch, after)? else {
            break;
        };
        let uid = source.uid;
        after = uid;
        if let Some(id) = source.upload
            && archive.accept_draft_upload(account, folder, epoch, uid, &id)?
        {
            continue;
        }
        let raw = blobs
            .get(&source.blob)
            .map_err(|e| SyncError::Draft(Box::new(e.into())))?;
        let draft = match decode(&raw, account, source.body, &attachments, &identities) {
            Ok(draft) => draft,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            },
        };
        archive.import_remote_draft(&draft, folder, epoch, uid)?;
    }
    let mut after = 0;
    for _ in 0..100 {
        check_cancel(cancelled)?;
        let Some((draft, revision)) = archive.dirty_draft(account, after)? else {
            break;
        };
        after = revision;
        let message_id = format!("{}@moonmail.local", uuid::Uuid::new_v4());
        let raw = match encode(&draft, &message_id, &attachments) {
            Ok(raw) => raw,
            Err(error) => {
                first_error.get_or_insert(error);
                continue;
            },
        };
        if !archive.claim_draft_upload(&draft, revision, folder, &message_id)? {
            continue;
        }
        match incoming
            .append_draft(folder, &raw)
            .map_err(SyncError::Incoming)?
        {
            DraftAppend::Saved { epoch, uid } => {
                archive.accept_draft_upload(account, folder, epoch, uid, &message_id)?;
            },
            DraftAppend::Unsupported => archive.unsupported_draft_upload(&message_id)?,
            DraftAppend::Unverified => {},
        }
    }
    if let Some(error) = first_error {
        return Err(SyncError::Draft(Box::new(error)));
    }
    Ok(())
}

pub(crate) fn deletions<I: IncomingMail>(
    incoming: &mut I,
    account: &str,
    archive: &mut Archive,
    cancelled: &impl Fn() -> bool,
) -> std::result::Result<(), SyncError<I::Error>> {
    loop {
        let queued = archive.remote_operations(Some(account))?;
        if queued.is_empty() {
            break;
        }
        for op in queued {
            check_cancel(cancelled)?;
            if !archive.claim_expunge(op.id)? {
                continue;
            }
            let result = incoming
                .expunge_uid(&op.mailbox, op.epoch, op.uid)
                .map_err(SyncError::Incoming)?;
            archive.finish_expunge(
                op.id,
                match result {
                    ExpungeOutcome::Removed => "done",
                    ExpungeOutcome::Stale => "stale",
                    ExpungeOutcome::Unsupported => "unsupported",
                },
            )?;
        }
    }
    Ok(())
}
