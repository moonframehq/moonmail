use {
    crate::normalize::normalize_bodies,
    mailparse::{DispositionType, MailAddr, MailHeaderMap, ParsedMail},
    moonmail_core::{MailboxAddress, MimeParser, ParsedMessage},
};

#[derive(Debug, thiserror::Error)]
pub enum MailparseError {
    #[error("MIME extraction failed")]
    Parse(#[from] mailparse::MailParseError),
}

#[derive(Default)]
pub struct MailparseParser;

impl MimeParser for MailparseParser {
    type Error = MailparseError;

    fn parse(&self, raw: &[u8]) -> Result<ParsedMessage, Self::Error> {
        let message = mailparse::parse_mail(raw)?;
        let mut parsed = ParsedMessage {
            date: message
                .headers
                .get_first_value("Date")
                .and_then(|date| mailparse::dateparse(&date).ok()),
            message_id: message_ids(&message, "Message-ID").into_iter().next(),
            references: message_ids(&message, "References"),
            in_reply_to: message_ids(&message, "In-Reply-To").into_iter().next(),
            subject: message.headers.get_first_value("Subject"),
            from: addresses(&message, "From")?,
            reply_to: addresses(&message, "Reply-To")?,
            to: addresses(&message, "To")?,
            cc: addresses(&message, "Cc")?,
            ..ParsedMessage::default()
        };
        extract_bodies(&message, &mut parsed)?;
        Ok(normalize_bodies(parsed))
    }
}

fn message_ids(message: &ParsedMail<'_>, header: &str) -> Vec<String> {
    message
        .headers
        .get_first_value(header)
        .and_then(|value| mailparse::msgidparse(&value).ok())
        .map(|ids| ids.iter().cloned().collect())
        .unwrap_or_default()
}

fn addresses(
    message: &ParsedMail<'_>,
    header: &str,
) -> Result<Vec<MailboxAddress>, MailparseError> {
    let Some(header) = message.headers.get_first_header(header) else {
        return Ok(Vec::new());
    };
    let parsed = mailparse::addrparse_header(header)?;
    let mut addresses = Vec::new();
    for address in parsed.iter() {
        match address {
            MailAddr::Single(single) => addresses.push(MailboxAddress {
                name: single.display_name.clone(),
                address: single.addr.clone(),
            }),
            MailAddr::Group(group) => {
                addresses.extend(group.addrs.iter().map(|single| MailboxAddress {
                    name: single.display_name.clone(),
                    address: single.addr.clone(),
                }));
            },
        }
    }
    Ok(addresses)
}

fn extract_bodies(part: &ParsedMail<'_>, output: &mut ParsedMessage) -> Result<(), MailparseError> {
    let disposition = part.get_content_disposition();
    if disposition.disposition == DispositionType::Attachment
        || disposition.params.contains_key("filename")
        || part.ctype.params.contains_key("name")
    {
        return Ok(());
    }
    match part.ctype.mimetype.as_str() {
        "text/plain" if output.text_body.is_none() => output.text_body = Some(part.get_body()?),
        "text/html" if output.html_body.is_none() => output.html_body = Some(part.get_body()?),
        mime if mime.starts_with("multipart/") => {
            for child in &part.subparts {
                extract_bodies(child, output)?;
            }
        },
        _ => {},
    }
    Ok(())
}
