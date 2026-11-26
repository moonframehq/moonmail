use {
    crate::{Error, InventoryBatch, RemoteMessage, ScanError, ScanOutcome},
    imap_proto::{AttributeValue, Response, ResponseCode, Status},
    moonmail_store::{InventoryKind, SyncCheckpoint, SyncFlags},
    std::io::{Read, Write},
};

pub(crate) fn uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

impl<S> From<Error> for ScanError<Error, S> {
    fn from(error: Error) -> Self {
        Self::Incoming(error)
    }
}
impl<S> From<imap::Error> for ScanError<Error, S> {
    fn from(error: imap::Error) -> Self {
        Self::Incoming(error.into())
    }
}

fn select<T: Read + Write>(
    session: &mut imap::Session<T>,
    folder: &str,
    condstore: bool,
) -> crate::Result<(u32, u32, Option<u64>)> {
    let folder = crate::imap_copy::mailbox(folder)?;
    let parameter = if condstore { " (CONDSTORE)" } else { "" };
    let data = session.run_command_and_read_response(format!("SELECT {folder}{parameter}"))?;
    let mut remaining = data.as_slice();
    let (mut epoch, mut next, mut modseq) = (None, None, None);
    while !remaining.is_empty() {
        let response = next_response(&mut remaining)?;
        if let Response::Data {
            status: Status::Ok,
            code: Some(code),
            ..
        } = response
        {
            match code {
                ResponseCode::UidValidity(n) => epoch = Some(n),
                ResponseCode::UidNext(n) => next = Some(n),
                ResponseCode::HighestModSeq(n) if condstore && n > 0 => modseq = Some(n),
                _ => {},
            }
        }
    }
    Ok((
        epoch
            .filter(|n| *n > 0)
            .ok_or(Error::Invalid("Server omitted UIDVALIDITY"))?,
        next.and_then(|n| n.checked_sub(1))
            .ok_or(Error::Invalid("Server omitted UIDNEXT"))?,
        modseq,
    ))
}

pub(crate) fn scan<T: Read + Write, S>(
    session: &mut imap::Session<T>,
    folder: &str,
    mut sink: impl FnMut(u32, u32, &[RemoteMessage]) -> Result<(), S>,
) -> Result<(), ScanError<Error, S>> {
    changes(
        session,
        folder,
        false,
        false,
        None,
        |epoch, upper, batch| match batch {
            InventoryBatch::Metadata(rows) => sink(epoch, upper, rows),
            _ => Ok(()),
        },
    )?;
    Ok(())
}

pub(crate) fn changes<T: Read + Write, S>(
    session: &mut imap::Session<T>,
    folder: &str,
    condstore: bool,
    qresync: bool,
    previous: Option<SyncCheckpoint>,
    mut sink: impl FnMut(u32, u32, InventoryBatch<'_>) -> Result<(), S>,
) -> Result<ScanOutcome, ScanError<Error, S>> {
    let (epoch, upper, modseq) = select(session, folder, condstore)?;
    let previous = previous.filter(|p| {
        p.epoch == epoch && p.upper <= upper && modseq.is_some_and(|n| n >= p.highest_modseq)
    });
    if let Some(base) = previous
        && base.upper == upper
        && Some(base.highest_modseq) == modseq
    {
        return Ok(ScanOutcome::Unchanged(base));
    }
    sink(epoch, upper, InventoryBatch::Metadata(&[])).map_err(ScanError::Sink)?;
    let mut emit = |batch: InventoryBatch<'_>| sink(epoch, upper, batch).map_err(ScanError::Sink);
    let mut lower = 0;
    let mut kind = InventoryKind::Full;
    if let Some(base) = previous {
        lower = base.upper;
        kind = if qresync {
            InventoryKind::Vanished(base)
        } else {
            InventoryKind::Membership(base)
        };
        if lower > 0 {
            let vanished = if qresync { " VANISHED" } else { "" };
            let response = session.run_command_and_read_response(format!(
                "UID FETCH 1:{lower} (UID FLAGS) (CHANGEDSINCE {}{vanished})",
                base.highest_modseq
            ))?;
            stage_changes(&response, lower, qresync, &mut emit)?;
        }
    }
    // SEARCH membership is required for expunges with plain CONDSTORE.
    // QRESYNC already supplied deletions, so search only the arrival interval.
    if upper > 0 && (!qresync || lower < upper) {
        let query = if qresync && lower > 0 {
            format!("UID {}:{upper}", lower + 1)
        } else {
            "ALL".to_owned()
        };
        let uids = search_uids(session, &query, upper)?;
        emit(InventoryBatch::Metadata(&[]))?;
        let split = uids.partition_point(|uid| *uid <= lower);
        if matches!(kind, InventoryKind::Membership(_)) {
            for batch in uids[..split].chunks(1000) {
                emit(InventoryBatch::Live(batch))?;
            }
        }
        // Even ten-digit UIDs fit below an 8 KiB command line.
        for batch in uids[split..].chunks(700) {
            let messages = metadata(session, batch)?;
            emit(InventoryBatch::Metadata(&messages))?;
        }
    }
    Ok(ScanOutcome::Complete { kind, modseq })
}

fn next_response<'a>(data: &mut &'a [u8]) -> crate::Result<Response<'a>> {
    let (rest, response) =
        Response::from_bytes(data).map_err(|_| Error::Invalid("Invalid inventory response"))?;
    *data = rest;
    Ok(response)
}

fn metadata<T: Read + Write>(
    session: &mut imap::Session<T>,
    batch: &[u32],
) -> crate::Result<Vec<RemoteMessage>> {
    let mut messages = Vec::new();
    for fetch in session
        .uid_fetch(uid_set(batch), "(UID FLAGS RFC822.SIZE INTERNALDATE)")?
        .iter()
    {
        let uid = fetch.uid.ok_or(Error::Invalid("Server omitted UID"))?;
        if batch.binary_search(&uid).is_err() {
            continue;
        }
        messages.push(RemoteMessage {
            uid,
            keywords: fetch
                .flags()
                .iter()
                .filter_map(|flag| match flag {
                    imap::types::Flag::Custom(s) if !s.starts_with('\\') => Some(s.to_string()),
                    _ => None,
                })
                .collect(),
            seen: fetch.flags().contains(&imap::types::Flag::Seen),
            starred: fetch.flags().contains(&imap::types::Flag::Flagged),
            size: fetch
                .size
                .ok_or(Error::Invalid("Server omitted message size"))?,
            received_at: fetch
                .internal_date()
                .ok_or(Error::Invalid("Invalid INTERNALDATE"))?
                .timestamp(),
        });
    }
    Ok(messages)
}

fn search_uids<T: Read + Write>(
    session: &mut imap::Session<T>,
    query: &str,
    upper: u32,
) -> crate::Result<Vec<u32>> {
    let response = session.run_command_and_read_response(format!("UID SEARCH {query}"))?;
    let mut data = response.as_slice();
    let mut uids = Vec::new();
    let mut received = false;
    while !data.is_empty() {
        if let Response::MailboxData(imap_proto::MailboxDatum::Search(ids)) =
            next_response(&mut data)?
        {
            received = true;
            uids.extend(ids.into_iter().filter(|uid| *uid > 0 && *uid <= upper));
        }
    }
    if !received {
        return Err(Error::Invalid("Server omitted SEARCH response"));
    }
    uids.sort_unstable();
    uids.dedup();
    Ok(uids)
}

fn stage_changes<S>(
    response: &[u8],
    lower: u32,
    qresync: bool,
    emit: &mut impl FnMut(InventoryBatch<'_>) -> Result<(), ScanError<Error, S>>,
) -> Result<(), ScanError<Error, S>> {
    let mut data = response;
    let mut changed = Vec::new();
    while !data.is_empty() {
        let response = next_response(&mut data)?;
        match response {
            Response::Fetch(_, attrs) => {
                let uid = attrs.iter().find_map(|a| {
                    if let AttributeValue::Uid(uid) = a {
                        Some(*uid)
                    } else {
                        None
                    }
                });
                let flags = attrs.iter().find_map(|a| {
                    if let AttributeValue::Flags(flags) = a {
                        Some(flags)
                    } else {
                        None
                    }
                });
                let uid = uid.ok_or(Error::Invalid("Server omitted changed UID"))?;
                if uid > 0 && uid <= lower {
                    let flags = flags.ok_or(Error::Invalid("Server omitted changed FLAGS"))?;
                    changed.push(SyncFlags {
                        uid,
                        seen: flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")),
                        starred: flags.iter().any(|f| f.eq_ignore_ascii_case("\\Flagged")),
                        keywords: flags
                            .iter()
                            .filter(|f| !f.starts_with('\\'))
                            .map(ToString::to_string)
                            .collect(),
                    });
                    if changed.len() == 1000 {
                        emit(InventoryBatch::Flags(&changed))?;
                        changed.clear();
                    }
                }
            },
            Response::Vanished { uids, .. } if qresync => {
                for uid in uids {
                    let lo = *uid.start().min(uid.end());
                    let hi = (*uid.end().max(uid.start())).min(lower);
                    if lo <= hi {
                        emit(InventoryBatch::Vanished(lo, hi))?;
                    }
                }
            },
            _ => {},
        }
    }
    emit(InventoryBatch::Flags(&changed))?;
    Ok(())
}
#[cfg(test)]
mod tests;
