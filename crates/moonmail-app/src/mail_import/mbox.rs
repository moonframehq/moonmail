//! Read one bounded record at a time. Length-framed bodies are opaque bytes.
//! Without Content-Length, use mboxrd quoting and one empty separator line.
use {
    super::LIMIT,
    anyhow::{Context, Result, ensure},
    std::io::{BufRead, Read},
};

pub(super) fn read(
    mut input: impl BufRead,
    mut emit: impl FnMut(&[u8]) -> Result<()>,
) -> Result<()> {
    let mut line = Vec::new();
    let mut message = Vec::new();
    read_line(&mut input, &mut line)?;
    while !line.is_empty() {
        ensure!(
            line.starts_with(b"From "),
            "Expected an mbox From separator"
        );
        ensure!(line.len() <= LIMIT, "Oversized mbox separator");
        message.clear();
        let mut length = None;
        loop {
            read_line(&mut input, &mut line)?;
            if line.is_empty() || line.starts_with(b"From ") {
                ensure!(length.is_none(), "Missing length-framed mbox body");
                break;
            }
            ensure!(
                message.len() + line.len() <= LIMIT,
                "Message exceeds 64 MiB"
            );
            message.extend_from_slice(&line);
            if matches!(line.as_slice(), b"\n" | b"\r\n") {
                if let Some(size) = length {
                    read_length_framed_body(&mut input, &mut message, size, &mut line)?;
                } else {
                    read_quoted_body(&mut input, &mut message, &mut line)?;
                }
                break;
            }
            if let Some((key, value)) = line.split_at_checked(b"Content-Length:".len())
                && key.eq_ignore_ascii_case(b"Content-Length:")
            {
                ensure!(length.is_none(), "Duplicate mbox Content-Length");
                let value = value.trim_ascii();
                ensure!(
                    value.iter().all(u8::is_ascii_digit),
                    "Invalid mbox Content-Length"
                );
                length = Some(
                    std::str::from_utf8(value)?
                        .parse::<usize>()
                        .context("Invalid mbox Content-Length")?,
                );
            }
        }
        ensure!(!message.is_empty(), "Empty mbox message");
        ensure!(message.len() <= LIMIT, "Message exceeds 64 MiB");
        emit(&message)?;
    }
    Ok(())
}

fn read_quoted_body(
    input: &mut impl BufRead,
    message: &mut Vec<u8>,
    line: &mut Vec<u8>,
) -> Result<()> {
    let mut padding = 0;
    loop {
        read_line(input, line)?;
        if line.is_empty() || line.starts_with(b"From ") {
            message.truncate(message.len() - padding);
            return Ok(());
        }
        // Only the last empty body line can be mbox framing.
        padding = if matches!(line.as_slice(), b"\n" | b"\r\n") {
            line.len()
        } else {
            0
        };
        let quoted = line.starts_with(b">")
            && line
                .iter()
                .position(|b| *b != b'>')
                .is_some_and(|i| line[i..].starts_with(b"From "));
        let payload = if quoted { &line[1..] } else { &line[..] };
        ensure!(
            message.len() + payload.len() <= LIMIT + 2,
            "Message exceeds 64 MiB"
        );
        message.extend_from_slice(payload);
    }
}

fn read_length_framed_body(
    input: &mut impl BufRead,
    message: &mut Vec<u8>,
    size: usize,
    line: &mut Vec<u8>,
) -> Result<()> {
    ensure!(
        size <= LIMIT.saturating_sub(message.len()),
        "Message exceeds 64 MiB"
    );
    let copied = input.take(size as u64).read_to_end(message)?;
    ensure!(copied == size, "Truncated length-framed mbox body");
    read_line(input, line)?;
    if !message.ends_with(b"\n") && !line.is_empty() {
        ensure!(
            matches!(line.as_slice(), b"\n" | b"\r\n"),
            "Invalid boundary after mbox Content-Length"
        );
        read_line(input, line)?;
    }
    if matches!(line.as_slice(), b"\n" | b"\r\n") {
        read_line(input, line)?;
    }
    ensure!(
        line.is_empty() || line.starts_with(b"From "),
        "Invalid boundary after mbox Content-Length"
    );
    Ok(())
}

fn read_line(input: &mut impl BufRead, line: &mut Vec<u8>) -> Result<()> {
    line.clear();
    input.take((LIMIT + 3) as u64).read_until(b'\n', line)?;
    Ok(())
}
