use {
    crate::{Error, RemoteMessage, ScanError},
    std::io::{Read, Write},
};

pub(crate) fn scan<T: Read + Write, S>(
    session: &mut imap::Session<T>,
    folder: &str,
    gmail: bool,
    mut sink: impl FnMut(u32, u32, &[RemoteMessage]) -> Result<(), S>,
) -> Result<(), ScanError<Error, S>> {
    let selected = session
        .select(folder)
        .map_err(|error| ScanError::Incoming(error.into()))?;
    let epoch = selected
        .uid_validity
        .filter(|epoch| *epoch > 0)
        .ok_or(ScanError::Incoming(Error::Invalid(
            "Server omitted UIDVALIDITY",
        )))?;
    let upper = selected
        .uid_next
        .filter(|next| *next > 0)
        .ok_or(ScanError::Incoming(Error::Invalid(
            "Server omitted UIDNEXT",
        )))?
        - 1;
    sink(epoch, upper, &[]).map_err(ScanError::Sink)?;
    // Fixed UID ranges avoid sequence shifts during EXPUNGE. Sparse UID spaces
    // cost extra commands, but require neither global SEARCH nor full FETCH.
    let mut lower = 1u64;
    while lower <= u64::from(upper) {
        let end = (lower + 999).min(u64::from(upper));
        let attributes = if gmail {
            "UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID X-GM-THRID X-GM-LABELS"
        } else {
            "UID FLAGS RFC822.SIZE INTERNALDATE"
        };
        let raw = session
            .run_command_and_read_response(format!("UID FETCH {lower}:{end} ({attributes})"))
            .map_err(|e| ScanError::Incoming(e.into()))?;
        let messages = parse_inventory(&raw, lower, end, gmail).map_err(ScanError::Incoming)?;
        sink(epoch, upper, &messages).map_err(ScanError::Sink)?;
        lower = end + 1;
    }
    Ok(())
}

fn parse_inventory(
    mut raw: &[u8],
    lower: u64,
    end: u64,
    gmail: bool,
) -> Result<Vec<RemoteMessage>, Error> {
    use imap_proto::{AttributeValue as A, Response};
    let mut batch = Vec::new();
    while !raw.is_empty() {
        let (rest, response) = imap_proto::parser::parse_response(raw)
            .map_err(|_| Error::Invalid("Malformed FETCH inventory"))?;
        raw = rest;
        let Response::Fetch(_, attributes) = response else {
            continue;
        };
        let (mut uid, mut size, mut received_at, mut flags) = (None, None, None, None);
        let (mut msgid, mut thrid, mut labels) = (None, None, None);
        for attr in attributes {
            match attr {
                A::Uid(v) => uid = Some(v),
                A::Rfc822Size(v) => size = Some(v),
                A::InternalDate(v) => {
                    received_at = chrono::DateTime::parse_from_str(&v, "%d-%b-%Y %H:%M:%S %z")
                        .ok()
                        .map(|d| d.timestamp())
                },
                A::Flags(v) => flags = Some(v),
                A::GmailMsgId(v) => msgid = Some(v),
                A::GmailThrId(v) => thrid = Some(v),
                A::GmailLabels(v) => {
                    labels = Some(
                        v.into_iter()
                            .map(|label| crate::imap_gmail::decode(&label))
                            .collect::<Result<_, _>>()?,
                    )
                },
                _ => {},
            }
        }
        // Servers may send unsolicited flag updates alongside a UID FETCH.
        if uid.is_none() && size.is_none() && received_at.is_none() {
            continue;
        }
        let uid = uid.ok_or(Error::Invalid("Server omitted UID"))?;
        if !(lower..=end).contains(&u64::from(uid)) {
            continue;
        }
        let flags = flags.ok_or(Error::Invalid("Server omitted FLAGS"))?;
        batch.push(RemoteMessage {
            uid,
            size: size.ok_or(Error::Invalid("Server omitted message size"))?,
            received_at: received_at.ok_or(Error::Invalid("Missing or invalid INTERNALDATE"))?,
            seen: flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")),
            starred: flags.iter().any(|f| f.eq_ignore_ascii_case("\\Flagged")),
            keywords: flags
                .into_iter()
                .filter(|f| !f.starts_with('\\'))
                .map(|f| f.into_owned())
                .collect(),
            gmail: if gmail {
                Some(moonmail_store::GmailMetadata {
                    msgid: msgid.ok_or(Error::Invalid("Missing X-GM-MSGID"))?,
                    thrid: thrid.ok_or(Error::Invalid("Missing X-GM-THRID"))?,
                    labels: labels.ok_or(Error::Invalid("Missing X-GM-LABELS"))?,
                })
            } else {
                None
            },
        });
    }
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{collections::VecDeque, io},
    };
    struct Script {
        steps: VecDeque<(&'static str, String)>,
        pending: Vec<u8>,
        response: VecDeque<u8>,
    }
    impl Read for Script {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let count = buf.len().min(self.response.len());
            for byte in &mut buf[..count] {
                *byte = self
                    .response
                    .pop_front()
                    .ok_or_else(|| io::Error::other("empty response"))?;
            }
            Ok(count)
        }
    }
    impl Write for Script {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.pending.extend(bytes);
            if self.pending.ends_with(b"\r\n") {
                let line = String::from_utf8(std::mem::take(&mut self.pending))
                    .map_err(io::Error::other)?;
                let (tag, command) = line
                    .trim_end()
                    .split_once(' ')
                    .ok_or_else(|| io::Error::other("missing command tag"))?;
                let (expected, response) = self
                    .steps
                    .pop_front()
                    .ok_or_else(|| io::Error::other("unexpected extra command"))?;
                if command != expected {
                    return Err(io::Error::other(format!(
                        "expected {expected}, got {command}"
                    )));
                }
                self.response.extend(response.replace("$TAG", tag).bytes());
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    fn session(last: &str) -> Result<imap::Session<Script>, Error> {
        let fetch = |uid| {
            format!(
                "* 1 FETCH (UID {uid} FLAGS (\\Seen $Label1 Work) RFC822.SIZE 120 INTERNALDATE \"21-Nov-1997 09:55:06 -0600\")\r\n"
            )
        };
        let script=Script {pending:Vec::new(),response:VecDeque::new(),steps:VecDeque::from([
            ("LOGIN \"test\" \"test\"","$TAG OK login\r\n".into()),
            ("SELECT \"INBOX\"","* 2 EXISTS\r\n* OK [UIDVALIDITY 7] epoch\r\n* OK [UIDNEXT 2002] next\r\n$TAG OK selected\r\n".into()),
            ("UID FETCH 1:1000 (UID FLAGS RFC822.SIZE INTERNALDATE)",fetch(1)+"$TAG OK first\r\n"),
            ("UID FETCH 1001:2000 (UID FLAGS RFC822.SIZE INTERNALDATE)","* 1 EXPUNGE\r\n$TAG OK empty range\r\n".into()),
            ("UID FETCH 2001:2001 (UID FLAGS RFC822.SIZE INTERNALDATE)",fetch(2001)+last),
        ])};
        imap::Client::new(script)
            .login("test", "test")
            .map_err(|(error, _)| error.into())
    }
    #[test]
    fn gmail_scan_and_label_store_use_extended_wire_commands()
    -> Result<(), Box<dyn std::error::Error>> {
        let select = "* 1 EXISTS\r\n* OK [UIDVALIDITY 7] epoch\r\n* OK [UIDNEXT 2] next\r\n$TAG OK [READ-WRITE] selected\r\n";
        let script=Script{pending:vec![],response:VecDeque::new(),steps:VecDeque::from([
            ("LOGIN \"test\" \"test\"","$TAG OK login\r\n".into()),
            ("SELECT \"INBOX\"",select.into()),
            ("UID FETCH 1:1 (UID FLAGS RFC822.SIZE INTERNALDATE X-GM-MSGID X-GM-THRID X-GM-LABELS)",
             "* 1 FETCH (UID 1 FLAGS () RFC822.SIZE 120 INTERNALDATE \"21-Nov-1997 09:55:06 -0600\" X-GM-MSGID 18446744073709551615 X-GM-THRID 42 X-GM-LABELS (\\Inbox \"Caf&AOk- plan\"))\r\n$TAG OK done\r\n".into()),
            ("SELECT \"INBOX\"",select.into()),
            ("UID FETCH 1 UID","* 1 FETCH (UID 1)\r\n$TAG OK found\r\n".into()),
            ("UID STORE 1 +X-GM-LABELS (\"Caf&AOk- plan\")","$TAG OK stored\r\n".into()),
            ("SELECT \"INBOX\"",select.into()),
            ("UID FETCH 1 UID","* 1 FETCH (UID 1)\r\n$TAG OK found\r\n".into()),
            ("UID STORE 1 -X-GM-LABELS (\\Inbox)","$TAG OK stored\r\n".into()),
        ])};
        let mut session = imap::Client::new(script)
            .login("test", "test")
            .map_err(|(e, _)| e)?;
        let mut found = Vec::new();
        scan(&mut session, "INBOX", true, |_, _, batch| {
            found.extend_from_slice(batch);
            Ok::<_, io::Error>(())
        })?;
        let gmail = found[0].gmail.as_ref().ok_or("missing metadata")?;
        assert_eq!((gmail.msgid, gmail.thrid), (u64::MAX, 42));
        assert_eq!(gmail.labels, ["\\Inbox", "Café plan"]);
        for (label, value) in [("Café plan", true), ("\\Inbox", false)] {
            assert_eq!(
                crate::imap_gmail::store(&mut session, "INBOX", 7, 1, label, value)?,
                crate::KeywordOutcome::Applied
            );
        }
        Ok(())
    }
    #[test]
    fn partial_gmail_metadata_cannot_complete_an_inventory() {
        let missing_labels=b"* 1 FETCH (UID 1 FLAGS () RFC822.SIZE 120 INTERNALDATE \"21-Nov-1997 09:55:06 -0600\" X-GM-MSGID 1 X-GM-THRID 2)\r\n";
        assert!(matches!(
            parse_inventory(missing_labels, 1, 1, true),
            Err(Error::Invalid("Missing X-GM-LABELS"))
        ));
    }

    #[test]
    fn native_scan_uses_fixed_uid_ranges_and_requires_final_tagged_success()
    -> Result<(), Box<dyn std::error::Error>> {
        for fail in [false, true] {
            let mut session = session(if fail {
                "$TAG NO interrupted\r\n"
            } else {
                "$TAG OK done\r\n"
            })?;
            let mut found = Vec::new();
            let mut sizes = Vec::new();
            let result = scan(&mut session, "INBOX", false, |epoch, upper, batch| {
                assert_eq!((epoch, upper), (7, 2001));
                sizes.push(batch.len());
                for message in batch {
                    assert_eq!(message.received_at, 880_127_706);
                    assert_eq!(message.keywords, ["$Label1", "Work"]);
                    found.push(message.uid);
                }
                Ok::<_, io::Error>(())
            });
            if fail {
                assert!(matches!(result, Err(ScanError::Incoming(_))));
                assert_eq!(found, vec![1]);
            } else {
                result?;
                assert_eq!(found, vec![1, 2001]);
                assert_eq!(sizes, vec![0, 1, 0, 1]);
            }
        }
        let mut session = session("$TAG OK done\r\n")?;
        let result = scan(&mut session, "INBOX", false, |_, _, batch| {
            if batch.is_empty() {
                Ok(())
            } else {
                Err(io::Error::other("cancelled"))
            }
        });
        assert!(matches!(result, Err(ScanError::Sink(_))));
        Ok(())
    }
}
