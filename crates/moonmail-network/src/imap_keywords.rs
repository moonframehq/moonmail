use {
    crate::{Error, KeywordOutcome, Result},
    imap::types::Flag,
    std::io::{Read, Write},
};

pub(crate) fn store<T: Read + Write>(
    session: &mut imap::Session<T>,
    folder: &str,
    epoch: u32,
    uid: u32,
    keyword: &str,
    value: bool,
) -> Result<KeywordOutcome> {
    let keyword =
        moonmail_core::canonical_keyword(keyword).ok_or(Error::Invalid("Invalid IMAP keyword"))?;
    if epoch == 0 || uid == 0 {
        return Err(Error::Invalid("Invalid keyword UID/UIDVALIDITY"));
    }
    let selected = session.select(folder)?;
    if selected.uid_validity != Some(epoch) {
        return Ok(KeywordOutcome::Stale);
    }
    // The pinned adapter conflates absent and empty PERMANENTFLAGS; require
    // explicit support rather than silently accepting session-only storage.
    if selected.is_read_only
        || !selected.permanent_flags.iter().any(|flag| match flag {
            Flag::MayCreate => true,
            Flag::Custom(name) => name.eq_ignore_ascii_case(&keyword),
            _ => false,
        })
    {
        return Ok(KeywordOutcome::Unsupported);
    }
    if !session
        .uid_fetch(uid.to_string(), "UID")?
        .iter()
        .any(|m| m.uid == Some(uid))
    {
        return Ok(KeywordOutcome::Stale);
    }
    session.uid_store(
        uid.to_string(),
        format!("{}FLAGS.SILENT ({keyword})", if value { "+" } else { "-" }),
    )?;
    Ok(KeywordOutcome::Applied)
}

#[cfg(test)]
mod tests {
    use {
        super::*,
        std::{collections::VecDeque, io},
    };
    struct Script {
        steps: VecDeque<(String, String)>,
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
                    .ok_or_else(|| io::Error::other("missing tag"))?;
                let (expected, response) = self
                    .steps
                    .pop_front()
                    .ok_or_else(|| io::Error::other("unexpected command"))?;
                assert_eq!(command, expected);
                self.response.extend(response.replace("$TAG", tag).bytes());
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn native_keyword_store_validates_epoch_uid_permanence_and_atom_before_mutation() -> Result<()>
    {
        for scenario in [
            "add",
            "remove",
            "named",
            "unsupported",
            "absent",
            "readonly",
            "epoch",
            "missing",
            "lost",
            "injection",
        ] {
            let mut steps =
                VecDeque::from([("LOGIN \"test\" \"test\"".into(), "$TAG OK login\r\n".into())]);
            if scenario != "injection" {
                let flags = match scenario {
                    "named" => "$LABEL1",
                    "unsupported" => "\\Seen",
                    "absent" => "",
                    _ => "\\*",
                };
                steps.push_back(("SELECT \"INBOX\"".into(),format!("* 1 EXISTS\r\n* OK [UIDVALIDITY {}] epoch\r\n* OK [PERMANENTFLAGS ({flags})] flags\r\n$TAG OK [{}] selected\r\n",if scenario=="epoch"{8}else{7},if scenario=="readonly"{"READ-ONLY"}else{"READ-WRITE"})));
                if !matches!(scenario, "unsupported" | "absent" | "readonly" | "epoch") {
                    steps.push_back((
                        "UID FETCH 3 UID".into(),
                        format!(
                            "{}$TAG OK found\r\n",
                            if scenario == "missing" {
                                ""
                            } else {
                                "* 1 FETCH (UID 3)\r\n"
                            }
                        ),
                    ));
                    if scenario != "missing" {
                        steps.push_back((
                            format!(
                                "UID STORE 3 {}FLAGS.SILENT ($label1)",
                                if scenario == "remove" { "-" } else { "+" }
                            ),
                            if scenario == "lost" {
                                String::new()
                            } else {
                                "$TAG OK stored\r\n".into()
                            },
                        ));
                    }
                }
            }
            let mut session = imap::Client::new(Script {
                steps,
                pending: vec![],
                response: VecDeque::new(),
            })
            .login("test", "test")
            .map_err(|(e, _)| Error::from(e))?;
            let outcome = store(
                &mut session,
                "INBOX",
                7,
                3,
                if scenario == "injection" {
                    "a)\r\nEXPUNGE"
                } else {
                    "$Label1"
                },
                scenario != "remove",
            );
            match scenario {
                "injection" | "lost" => assert!(outcome.is_err(), "{scenario}"),
                "unsupported" | "absent" | "readonly" => {
                    assert_eq!(outcome?, KeywordOutcome::Unsupported, "{scenario}")
                },
                "epoch" | "missing" => assert_eq!(outcome?, KeywordOutcome::Stale, "{scenario}"),
                _ => assert_eq!(outcome?, KeywordOutcome::Applied, "{scenario}"),
            }
        }
        Ok(())
    }
}
