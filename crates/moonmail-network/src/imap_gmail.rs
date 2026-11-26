use {
    crate::{Error, KeywordOutcome, Result},
    base64::{Engine, engine::general_purpose::STANDARD_NO_PAD},
    std::io::{Read, Write},
};

// IMAP mailbox names and Gmail user labels use modified UTF-7.
fn encode(label: &str) -> String {
    let mut wire = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            wire.push_str("&-");
        } else if c.is_ascii() {
            wire.push(c);
        } else {
            let mut run = String::from(c);
            while let Some(next) = chars.next_if(|c| !c.is_ascii()) {
                run.push(next);
            }
            let bytes: Vec<_> = run.encode_utf16().flat_map(u16::to_be_bytes).collect();
            wire.push('&');
            wire.push_str(&STANDARD_NO_PAD.encode(bytes).replace('/', ","));
            wire.push('-');
        }
    }
    wire
}

pub(crate) fn decode(label: &str) -> Result<String> {
    let mut text = String::new();
    // imap-proto retains the quoted string's two permitted escape sequences.
    let mut unquoted = String::new();
    let mut chars = label.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(escaped) = chars.next_if(|v| matches!(v, '\\' | '"'))
        {
            unquoted.push(escaped);
        } else {
            unquoted.push(c);
        }
    }
    let mut tail = unquoted.as_str();
    while let Some((prefix, encoded)) = tail.split_once('&') {
        text.push_str(prefix);
        let (encoded, rest) = encoded
            .split_once('-')
            .ok_or(Error::Invalid("Invalid modified UTF-7 label"))?;
        tail = rest;
        if encoded.is_empty() {
            text.push('&');
            continue;
        }
        let bytes = STANDARD_NO_PAD
            .decode(encoded.replace(',', "/"))
            .map_err(|_| Error::Invalid("Invalid modified UTF-7 label"))?;
        let (units, remainder) = bytes.as_chunks::<2>();
        if !remainder.is_empty() {
            return Err(Error::Invalid("Incomplete UTF-16 label"));
        }
        for c in char::decode_utf16(units.iter().copied().map(u16::from_be_bytes)) {
            text.push(c.map_err(|_| Error::Invalid("Invalid UTF-16 label"))?);
        }
    }
    text.push_str(tail);
    Ok(text)
}

pub(crate) fn store<T: Read + Write>(
    session: &mut imap::Session<T>,
    folder: &str,
    epoch: u32,
    uid: u32,
    label: &str,
    value: bool,
) -> Result<KeywordOutcome> {
    if !moonmail_store::valid_gmail_label(label) || epoch == 0 || uid == 0 {
        return Err(Error::Invalid("Invalid Gmail label operation"));
    }
    let selected = session.select(folder)?;
    if selected.uid_validity != Some(epoch) {
        return Ok(KeywordOutcome::Stale);
    }
    if selected.is_read_only {
        return Ok(KeywordOutcome::Unsupported);
    }
    if !session
        .uid_fetch(uid.to_string(), "UID")?
        .iter()
        .any(|f| f.uid == Some(uid))
    {
        return Ok(KeywordOutcome::Stale);
    }
    let wire = if label
        .strip_prefix('\\')
        .is_some_and(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphabetic()))
    {
        label.to_owned()
    } else {
        format!(
            "\"{}\"",
            encode(label).replace('\\', "\\\\").replace('"', "\\\"")
        )
    };
    match session.uid_store(
        uid.to_string(),
        format!("{}X-GM-LABELS ({wire})", if value { "+" } else { "-" }),
    ) {
        Ok(_) => Ok(KeywordOutcome::Applied),
        Err(imap::Error::No(_) | imap::Error::Bad(_)) => Ok(KeywordOutcome::Unsupported),
        Err(error) => Err(error.into()),
    }
}
