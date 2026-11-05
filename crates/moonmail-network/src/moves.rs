use {
    crate::{CopyOutcome, IncomingMail, MoveOutcome, SyncError},
    moonmail_store::{Archive, MoveOperation},
};

pub(crate) fn apply_moves<I: IncomingMail>(
    incoming: &mut I,
    account: &str,
    archive: &mut Archive,
    progress: &mut impl FnMut(String),
    cancelled: &impl Fn() -> bool,
) -> Result<(), SyncError<I::Error>> {
    use moonmail_core::MailRepository;
    let folders = archive.mailboxes()?;
    for operation in archive.pending_moves(account)? {
        crate::sync::check_cancel(cancelled)?;
        if !folders
            .iter()
            .any(|folder| folder.id == operation.source_id && folder.selectable)
            || !folders
                .iter()
                .any(|folder| folder.id == operation.destination_id && folder.selectable)
        {
            archive.fail_queued_move(
                operation.id,
                "Source or destination disappeared; choose an active folder",
            )?;
            continue;
        }
        // Nothing mutating has been dispatched: a failed SELECT leaves queued
        if archive.has_keyword_intent(operation.source_id, operation.epoch, operation.uid)? {
            archive.fail_queued_move(
                operation.id,
                "Resolve pending or blocked keyword edits before transferring this message",
            )?;
            continue;
        }
        // Nothing mutating has been dispatched: a failed SELECT leaves queued
        // intent retryable on the next connection.
        let source = incoming
            .select_uid(&operation.source, operation.uid)
            .map_err(SyncError::Incoming)?;
        if source.epoch != operation.epoch || !source.exists {
            archive.fail_queued_move(
                operation.id,
                "Source UID/UIDVALIDITY changed; move was not sent",
            )?;
            continue;
        }
        // Preserve all durable flags that preceded the move. New flag edits are
        // excluded while a move is queued/running/unknown.
        for flag in archive.pending_flags(account)?.into_iter().filter(|flag| {
            flag.mailbox == operation.source
                && flag.epoch == operation.epoch
                && flag.uid == operation.uid
        }) {
            crate::sync::check_cancel(cancelled)?;
            incoming
                .flag(flag.uid, &flag.kind, flag.value)
                .map_err(SyncError::Incoming)?;
            archive.acknowledge_flag(&flag)?;
        }
        progress(format!(
            "{} message to {}",
            if operation.kind == "copy" {
                "Copying"
            } else {
                "Moving"
            },
            operation.destination
        ));
        crate::sync::check_cancel(cancelled)?;
        if !archive.claim_move(operation.id)? {
            continue;
        }
        if operation.kind == "copy" {
            match incoming.copy_message(operation.uid,&operation.destination) {
                Ok(CopyOutcome::Unsupported)=>archive.finish_move(operation.id,"failed","Adapter does not support COPY; no copy was sent")?,
                Ok(CopyOutcome::Copied{epoch,uid})=>{
                    archive.record_move_copy(operation.id,epoch,uid)?;
                    archive.finish_move(operation.id,"done","Server acknowledged COPY; source retained and destination refresh follows")?;
                },
                Ok(CopyOutcome::Unverified)=>archive.finish_move(operation.id,"done","Server acknowledged COPY without a UID mapping; source retained and destination refresh follows")?,
                Err(error)=>{
                    archive.finish_move(operation.id,"unknown","COPY response failed or was lost. Source retained; sync and inspect the destination before dismissing. No automatic retry.")?;
                    return Err(SyncError::Incoming(error));
                },
            }
            continue;
        }
        match incoming.move_message(operation.uid, &operation.destination) {
            Ok(MoveOutcome::Moved) => archive.finish_move(
                operation.id,
                "done",
                "Server acknowledged UID MOVE; destination refresh follows",
            )?,
            Ok(MoveOutcome::Unsupported) => {
                if cancelled() {
                    archive.finish_move(
                        operation.id,
                        "failed",
                        "Stopped before COPY; no move was sent",
                    )?;
                    return Err(SyncError::Cancelled);
                }
                let result = copy_move(incoming, archive, &operation, cancelled);
                match result {
                    Ok((state, detail)) => archive.finish_move(operation.id, state, detail)?,
                    Err(SyncError::Incoming(error)) => {
                        archive.finish_move(operation.id, "unknown", "Copy fallback interrupted. Source removal may be incomplete; sync and inspect both folders. No automatic retry.")?;
                        return Err(SyncError::Incoming(error));
                    },
                    Err(SyncError::Cancelled) => {
                        archive.finish_move(operation.id, "unknown", "Stopped after copying; source was retained. Inspect both folders before dismissing.")?;
                        return Err(SyncError::Cancelled);
                    },
                    Err(error) => return Err(error),
                }
            },
            Err(error) => {
                archive.finish_move(operation.id,"unknown","MOVE response was lost or failed. Sync and inspect both folders before dismissing; no automatic retry.")?;
                return Err(SyncError::Incoming(error));
            },
        }
    }
    Ok(())
}

fn copy_move<I: IncomingMail>(
    incoming: &mut I,
    archive: &Archive,
    operation: &MoveOperation,
    cancelled: &impl Fn() -> bool,
) -> Result<(&'static str, &'static str), SyncError<I::Error>> {
    let receipt = incoming
        .copy_for_move(operation.uid, &operation.destination)
        .map_err(SyncError::Incoming)?;
    let (epoch, uid) = match receipt {
        CopyOutcome::Unsupported => {
            return Ok((
                "failed",
                "Server supports neither MOVE nor a UIDPLUS copy fallback; no copy/delete command was sent",
            ));
        },
        CopyOutcome::Unverified => {
            return Ok((
                "unknown",
                "COPY acknowledged without a usable destination UID mapping. Source retained; inspect both folders.",
            ));
        },
        CopyOutcome::Copied { epoch, uid } => (epoch, uid),
    };
    archive.record_move_copy(operation.id, epoch, uid)?;
    crate::sync::check_cancel(cancelled)?;
    let destination = incoming
        .select_uid(&operation.destination, uid)
        .map_err(SyncError::Incoming)?;
    if destination.epoch != epoch || !destination.exists {
        return Ok((
            "unknown",
            "Copied destination UID could not be verified. Source retained; inspect both folders.",
        ));
    }
    let expected = archive.message_blob(operation.message_id)?;
    if incoming
        .message_digest(uid)
        .map_err(SyncError::Incoming)?
        .as_deref()
        != Some(expected.as_str())
    {
        return Ok((
            "unknown",
            "Destination bytes differ, are missing, or exceed the verification limit. Source retained; inspect both folders.",
        ));
    }
    crate::sync::check_cancel(cancelled)?;
    let source = incoming
        .select_uid(&operation.source, operation.uid)
        .map_err(SyncError::Incoming)?;
    if source.epoch != operation.epoch {
        return Ok((
            "unknown",
            "Destination verified, but source UIDVALIDITY changed. Source retained; inspect both folders.",
        ));
    }
    crate::sync::check_cancel(cancelled)?;
    if source.exists
        && matches!(
            incoming
                .remove_copied_source(operation.uid)
                .map_err(SyncError::Incoming)?,
            MoveOutcome::Unsupported
        )
    {
        return Ok((
            "unknown",
            "Destination verified, but targeted source removal is unavailable. Source retained; inspect both folders.",
        ));
    }
    // No cancellation boundary may separate source removal from durable outcome.
    Ok((
        "done",
        "Destination bytes verified; source UID removed. Destination refresh follows.",
    ))
}
