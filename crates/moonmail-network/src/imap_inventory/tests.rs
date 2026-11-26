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
            let line =
                String::from_utf8(std::mem::take(&mut self.pending)).map_err(io::Error::other)?;
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
            ("UID SEARCH ALL","* SEARCH 1 2001 2002\r\n$TAG OK ids\r\n".into()),
            ("UID FETCH 1,2001 (UID FLAGS RFC822.SIZE INTERNALDATE)",fetch(1)+&fetch(2001)+last),
        ])};
    imap::Client::new(script)
        .login("test", "test")
        .map_err(|(error, _)| error.into())
}
#[test]
fn native_scan_skips_sparse_uid_holes_and_requires_final_tagged_success()
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
            assert!(found.is_empty());
        } else {
            result?;
            assert_eq!(found, vec![1, 2001]);
            assert_eq!(sizes, vec![0, 0, 2]);
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

fn scripted(steps: Vec<(&'static str, String)>) -> Result<imap::Session<Script>, Error> {
    let mut steps: VecDeque<_> = steps.into();
    steps.push_front(("LOGIN \"test\" \"test\"", "$TAG OK login\r\n".into()));
    imap::Client::new(Script {
        steps,
        pending: Vec::new(),
        response: VecDeque::new(),
    })
    .login("test", "test")
    .map_err(|(e, _)| e.into())
}

fn selected(epoch: u32, next: u32, code: &str) -> (&'static str, String) {
    (
        "SELECT \"INBOX\" (CONDSTORE)",
        format!(
            "* OK [UIDVALIDITY {epoch}] epoch\r\n* OK [UIDNEXT {next}] next\r\n* OK [{code}] modseq state\r\n$TAG OK done\r\n"
        ),
    )
}

#[test]
fn unchanged_selection_skips_all_fetches_and_epoch_reset_or_nomodseq_scans()
-> Result<(), Box<dyn std::error::Error>> {
    let base = SyncCheckpoint {
        epoch: 7,
        upper: 2,
        highest_modseq: 5,
    };
    let mut imap = scripted(vec![selected(7, 3, "HIGHESTMODSEQ 5")])?;
    assert!(
        matches!(changes(&mut imap,"INBOX",true,false,Some(base),|_,_,_|Err(io::Error::other("unexpected scan")))?,ScanOutcome::Unchanged(value) if value==base)
    );
    for (epoch, code) in [(8, "HIGHESTMODSEQ 5"), (7, "NOMODSEQ")] {
        let mut imap = scripted(vec![
            selected(epoch, 3, code),
            ("UID SEARCH ALL", "* SEARCH\r\n$TAG OK ids\r\n".into()),
        ])?;
        assert!(matches!(
            changes(&mut imap, "INBOX", true, false, Some(base), |_, _, _| Ok::<
                _,
                io::Error,
            >(
                ()
            ))?,
            ScanOutcome::Complete {
                kind: InventoryKind::Full,
                ..
            }
        ));
    }
    Ok(())
}

#[test]
fn qresync_stages_changed_flags_vanished_and_new_metadata() -> Result<(), Box<dyn std::error::Error>>
{
    let mut imap=scripted(vec![
        selected(7,4,"HIGHESTMODSEQ 9"),
        ("UID FETCH 1:2 (UID FLAGS) (CHANGEDSINCE 5 VANISHED)","* VANISHED (EARLIER) 1\r\n* 1 FETCH (UID 2 FLAGS (\\Flagged Work) MODSEQ (8))\r\n$TAG OK changed\r\n".into()),
        ("UID SEARCH UID 3:3","* SEARCH 3\r\n$TAG OK ids\r\n".into()),
        ("UID FETCH 3 (UID FLAGS RFC822.SIZE INTERNALDATE)","* 2 FETCH (UID 3 FLAGS () RFC822.SIZE 10 INTERNALDATE \"21-Nov-1997 09:55:06 -0600\")\r\n$TAG OK done\r\n".into()),
    ])?;
    let base = SyncCheckpoint {
        epoch: 7,
        upper: 2,
        highest_modseq: 5,
    };
    let mut events = Vec::new();
    changes(&mut imap, "INBOX", true, true, Some(base), |_, _, batch| {
        match batch {
            InventoryBatch::Vanished(1, 1) => events.push("expunge"),
            InventoryBatch::Flags([flags])
                if flags.uid == 2 && flags.starred && flags.keywords == ["Work"] =>
            {
                events.push("flags")
            },
            InventoryBatch::Metadata([m]) if m.uid == 3 => events.push("arrival"),
            _ => {},
        }
        Ok::<_, io::Error>(())
    })?;
    assert_eq!(events, ["expunge", "flags", "arrival"]);
    Ok(())
}
