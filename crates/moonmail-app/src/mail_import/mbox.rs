//! Streaming mboxrd. An unquoted From_ line delimits messages. Within a
//! message one leading '>' is removed from quoted From_ lines.
use {
    anyhow::{Result, ensure},
    std::io::{BufRead, Read},
};

pub(super) fn read(
    mut input: impl BufRead,
    mut emit: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let mut line = Vec::new();
    let mut message = Vec::new();
    let mut started = false;
    loop {
        line.clear();
        let n = (&mut input)
            .take((super::LIMIT + 3) as u64)
            .read_until(b'\n', &mut line)?;
        let boundary = line.starts_with(b"From ");
        if n == 0 || boundary {
            if started {
                // mbox separates messages with an additional empty line.
                if message.ends_with(b"\r\n\r\n") {
                    message.truncate(message.len() - 2);
                } else if message.ends_with(b"\n\n") {
                    message.pop();
                }
                ensure!(!message.is_empty(), "Empty mbox message");
                emit(&message)?;
                message.clear();
            }
            if n == 0 {
                return Ok(());
            }
            ensure!(n <= super::LIMIT, "Oversized mbox separator");
            started = true;
        } else {
            ensure!(started, "Expected an mbox From separator");
            let quoted = line.first() == Some(&b'>')
                && line
                    .iter()
                    .position(|b| *b != b'>')
                    .is_some_and(|i| line[i..].starts_with(b"From "));
            let payload = if quoted { &line[1..] } else { &line };
            ensure!(
                message.len() + payload.len() <= super::LIMIT + 2,
                "Message exceeds 64 MiB"
            );
            message.extend_from_slice(payload);
        }
    }
}
