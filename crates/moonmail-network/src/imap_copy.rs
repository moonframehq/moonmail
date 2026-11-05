use {
    crate::{CopyOutcome, Error, MoveOutcome, Result},
    imap_proto::{Response, ResponseCode, Status, UidSetMember},
    sha2::{Digest, Sha256},
    std::io::{Read, Write},
};

fn mailbox(name: &str) -> Result<String> {
    if name.is_empty() || name.chars().any(char::is_control) {
        return Err(Error::Invalid("Invalid copy destination"));
    }
    Ok(format!(
        "\"{}\"",
        name.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

fn single(values: &[UidSetMember]) -> Option<u32> {
    match values {
        [UidSetMember::Uid(uid)] if *uid != 0 => Some(*uid),
        [UidSetMember::UidRange(range)] if range.start() == range.end() && *range.start() != 0 => {
            Some(*range.start())
        },
        _ => None,
    }
}

fn receipt(response: &[u8], source: u32) -> CopyOutcome {
    if let Ok((
        rest,
        Response::Done {
            status: Status::Ok,
            code: Some(ResponseCode::CopyUid(epoch, from, to)),
            ..
        },
    )) = imap_proto::parser::parse_response(response)
        && rest.is_empty()
        && epoch != 0
        && single(&from) == Some(source)
        && let Some(uid) = single(&to)
    {
        return CopyOutcome::Copied { epoch, uid };
    }
    CopyOutcome::Unverified
}

pub(crate) fn copy<T: Read + Write>(
    session: &mut imap::Session<T>,
    uid: u32,
    destination: &str,
    require_uidplus: bool,
) -> Result<CopyOutcome> {
    let destination = mailbox(destination)?;
    if require_uidplus && !session.capabilities()?.has_str("UIDPLUS") {
        return Ok(CopyOutcome::Unsupported);
    }
    // run includes the tagged OK and its COPYUID code; uid_copy discards it.
    let (response, final_start) = session.run(format!("UID COPY {uid} {destination}"))?;
    Ok(receipt(&response[final_start..], uid))
}

pub(crate) fn digest<T: Read + Write>(
    session: &mut imap::Session<T>,
    uid: u32,
) -> Result<Option<String>> {
    const LIMIT: u32 = 64 * 1024 * 1024;
    let metadata = session.uid_fetch(uid.to_string(), "(UID RFC822.SIZE)")?;
    let Some(size) = metadata
        .iter()
        .find(|item| item.uid == Some(uid))
        .and_then(|item| item.size)
        .filter(|size| *size <= LIMIT)
    else {
        return Ok(None);
    };
    // A partial request bounds the literal even if the earlier size is wrong.
    let messages = session.uid_fetch(
        uid.to_string(),
        format!("(UID BODY.PEEK[]<0.{}>)", LIMIT + 1),
    )?;
    Ok(messages
        .iter()
        .find(|item| item.uid == Some(uid))
        .and_then(|item| item.body())
        .filter(|raw| raw.len() == size as usize && raw.len() <= LIMIT as usize)
        .map(|raw| {
            Sha256::digest(raw)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect()
        }))
}

pub(crate) fn remove<T: Read + Write>(
    session: &mut imap::Session<T>,
    uid: u32,
) -> Result<MoveOutcome> {
    if !session.capabilities()?.has_str("UIDPLUS") {
        return Ok(MoveOutcome::Unsupported);
    }
    session.uid_store(uid.to_string(), "+FLAGS.SILENT (\\Deleted)")?;
    session.uid_expunge(uid.to_string())?;
    Ok(MoveOutcome::Moved)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{cell::Cell, collections::VecDeque, io, rc::Rc},
    };

    struct Wire {
        steps: VecDeque<(&'static str, String)>,
        pending: Vec<u8>,
        response: VecDeque<u8>,
        remaining: Rc<Cell<usize>>,
    }

    impl Read for Wire {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            let count = bytes.len().min(self.response.len());
            for byte in &mut bytes[..count] {
                *byte = self
                    .response
                    .pop_front()
                    .ok_or_else(|| io::Error::other("empty response"))?;
            }
            Ok(count)
        }
    }
    impl Write for Wire {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.pending.extend(bytes);
            if self.pending.ends_with(b"\r\n") {
                let line = String::from_utf8(std::mem::take(&mut self.pending))
                    .map_err(io::Error::other)?;
                let (tag, command) = line
                    .trim_end()
                    .split_once(' ')
                    .ok_or_else(|| io::Error::other("missing tag"))?;
                let (expected, response) = self
                    .steps
                    .pop_front()
                    .ok_or_else(|| io::Error::other("unexpected command"))?;
                assert_eq!(command, expected);
                self.remaining.set(self.steps.len());
                self.response.extend(response.replace("$TAG", tag).bytes());
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn session(
        steps: Vec<(&'static str, String)>,
    ) -> Result<(imap::Session<Wire>, Rc<Cell<usize>>)> {
        let mut steps: VecDeque<_> = steps.into();
        steps.push_front(("LOGIN \"test\" \"test\"", "$TAG OK login\r\n".into()));
        let remaining = Rc::new(Cell::new(steps.len()));
        let wire = Wire {
            steps,
            pending: Vec::new(),
            response: VecDeque::new(),
            remaining: remaining.clone(),
        };
        Ok((
            imap::Client::new(wire)
                .login("test", "test")
                .map_err(|(error, _)| error)?,
            remaining,
        ))
    }

    #[test]
    fn native_copy_verifies_bounded_literal_and_uses_only_uid_targeted_expunge() -> Result<()> {
        let capabilities = "* CAPABILITY IMAP4rev1 UIDPLUS\r\n$TAG OK capabilities\r\n".to_owned();
        let (mut s, remaining) = session(vec![
            ("CAPABILITY", capabilities.clone()),
            (
                "UID COPY 12 \"Projects/Review notes\"",
                "$TAG OK [COPYUID 7 12 34] copied\r\n".into(),
            ),
            (
                "UID FETCH 34 (UID RFC822.SIZE)",
                "* 1 FETCH (UID 34 RFC822.SIZE 5)\r\n$TAG OK metadata\r\n".into(),
            ),
            (
                "UID FETCH 34 (UID BODY.PEEK[]<0.67108865>)",
                "* 1 FETCH (UID 34 BODY[]<0> {5}\r\nhello)\r\n$TAG OK body\r\n".into(),
            ),
            ("CAPABILITY", capabilities),
            (
                "UID STORE 12 +FLAGS.SILENT (\\Deleted)",
                "$TAG OK deleted\r\n".into(),
            ),
            (
                "UID EXPUNGE 12",
                "* 1 EXPUNGE\r\n$TAG OK expunged\r\n".into(),
            ),
        ])?;
        assert!(matches!(
            copy(&mut s, 12, "Projects/Review notes", true)?,
            CopyOutcome::Copied { epoch: 7, uid: 34 }
        ));
        assert_eq!(
            digest(&mut s, 34)?.as_deref(),
            Some("2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824")
        );
        assert!(matches!(remove(&mut s, 12)?, MoveOutcome::Moved));
        assert_eq!(remaining.get(), 0);
        let (mut s, remaining) = session(vec![
            (
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1\r\n$TAG OK capabilities\r\n".into(),
            ),
            (
                "CAPABILITY",
                "* CAPABILITY IMAP4rev1\r\n$TAG OK capabilities\r\n".into(),
            ),
            (
                "UID FETCH 34 (UID RFC822.SIZE)",
                "* 1 FETCH (UID 34 RFC822.SIZE 67108865)\r\n$TAG OK metadata\r\n".into(),
            ),
        ])?;
        assert!(matches!(
            copy(&mut s, 12, "Destination", true)?,
            CopyOutcome::Unsupported
        ));
        assert!(matches!(remove(&mut s, 12)?, MoveOutcome::Unsupported));
        assert!(digest(&mut s, 34)?.is_none());
        assert_eq!(remaining.get(), 0);
        Ok(())
    }

    #[test]
    fn copy_receipts_require_exact_single_uid_mapping_and_folder_names_are_quoted() -> Result<()> {
        assert_eq!(
            mailbox("Projects/Review \"A\"\\B")?,
            "\"Projects/Review \\\"A\\\"\\\\B\""
        );
        assert!(mailbox("INBOX\r\nEXPUNGE").is_err());
        assert!(matches!(
            receipt(b"a1 OK [COPYUID 7 12 34] Copied\r\n", 12),
            CopyOutcome::Copied { epoch: 7, uid: 34 }
        ));
        assert!(matches!(
            receipt(b"a1 OK [COPYUID 7 12:12 34:34] Copied\r\n", 12),
            CopyOutcome::Copied { epoch: 7, uid: 34 }
        ));
        for response in [
            &b"a1 OK Copied\r\n"[..],
            &b"a1 OK [COPYUID 7 13 34] Wrong source\r\n"[..],
            &b"a1 OK [COPYUID 7 12:14 34:36] Multiple\r\n"[..],
            &b"a1 OK [COPYUID 7 12 34,35] Multiple\r\n"[..],
            &b"a1 NO [COPYUID 7 12 34] Rejected\r\n"[..],
            &b"a1 OK [COPYUID 7 12 34] Copied\r\ntrailing"[..],
        ] {
            assert!(matches!(receipt(response, 12), CopyOutcome::Unverified));
        }
        Ok(())
    }

    #[test]
    fn native_folder_mutations_quote_names_and_deselect_without_expunge() -> Result<()> {
        use moonmail_core::FolderChange;
        let selected =
            "* 0 EXISTS\r\n* OK [UIDVALIDITY 7] epoch\r\n$TAG OK examined\r\n".to_owned();
        let (mut s, remaining) = session(vec![
            ("CREATE \"Review notes\"", "$TAG OK created\r\n".into()),
            ("EXAMINE \"INBOX\"", selected.clone()),
            (
                "RENAME \"Review notes\" \"Work \\\"quoted\\\"\"",
                "$TAG OK renamed\r\n".into(),
            ),
            ("EXAMINE \"INBOX\"", selected),
            (
                "DELETE \"Work \\\"quoted\\\"\"",
                "$TAG OK deleted\r\n".into(),
            ),
        ])?;
        crate::imap_folders::edit(
            &mut s,
            &FolderChange::Create {
                name: "Review notes".into(),
            },
        )?;
        crate::imap_folders::edit(
            &mut s,
            &FolderChange::Rename {
                source: "Review notes".into(),
                destination: "Work \"quoted\"".into(),
            },
        )?;
        crate::imap_folders::edit(
            &mut s,
            &FolderChange::Delete {
                name: "Work \"quoted\"".into(),
            },
        )?;
        assert_eq!(remaining.get(), 0);
        Ok(())
    }

    #[test]
    fn standalone_copy_needs_no_uidplus_and_accepts_acknowledgment_without_mapping() -> Result<()> {
        for response in ["$TAG OK copied\r\n", "$TAG OK [COPYUID 9 12 24] copied\r\n"] {
            let (mut s, remaining) =
                session(vec![("UID COPY 12 \"Destination\"", response.into())])?;
            let outcome = copy(&mut s, 12, "Destination", false)?;
            if response.contains("COPYUID") {
                assert!(matches!(outcome, CopyOutcome::Copied { epoch: 9, uid: 24 }));
            } else {
                assert!(matches!(outcome, CopyOutcome::Unverified));
            }
            assert_eq!(remaining.get(), 0);
        }
        Ok(())
    }
}
