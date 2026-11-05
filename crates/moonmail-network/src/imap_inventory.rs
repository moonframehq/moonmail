use {
    crate::{Error, RemoteMessage, ScanError},
    std::io::{Read, Write},
};

pub(crate) fn scan<T: Read + Write, S>(
    session: &mut imap::Session<T>,
    folder: &str,
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
        let mut messages = Vec::new();
        for fetch in session
            .uid_fetch(
                format!("{lower}:{end}"),
                "(UID FLAGS RFC822.SIZE INTERNALDATE)",
            )
            .map_err(|error| ScanError::Incoming(error.into()))?
            .iter()
        {
            let uid = fetch
                .uid
                .ok_or(ScanError::Incoming(Error::Invalid("Server omitted UID")))?;
            if u64::from(uid) < lower || u64::from(uid) > end {
                continue;
            }
            messages.push(RemoteMessage {
                keywords: fetch
                    .flags()
                    .iter()
                    .filter_map(|flag| match flag {
                        imap::types::Flag::Custom(value) if !value.starts_with('\\') => {
                            Some(value.to_string())
                        },
                        _ => None,
                    })
                    .collect(),
                uid,
                seen: fetch.flags().contains(&imap::types::Flag::Seen),
                starred: fetch.flags().contains(&imap::types::Flag::Flagged),
                size: fetch.size.ok_or(ScanError::Incoming(Error::Invalid(
                    "Server omitted message size",
                )))?,
                received_at: fetch
                    .internal_date()
                    .ok_or(ScanError::Incoming(Error::Invalid(
                        "Server omitted or sent an invalid INTERNALDATE",
                    )))?
                    .timestamp(),
            });
        }
        sink(epoch, upper, &messages).map_err(ScanError::Sink)?;
        lower = end + 1;
    }
    Ok(())
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
            let result = scan(&mut session, "INBOX", |epoch, upper, batch| {
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
        let result = scan(&mut session, "INBOX", |_, _, batch| {
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
