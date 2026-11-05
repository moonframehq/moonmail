use {
    crate::MailparseParser,
    mailparse::{DispositionType, MailHeaderMap, ParsedMail},
    moonmail_core::{Attachment, MimeAttachments},
};

const MAX_RAW: usize = 64 * 1024 * 1024;
const MAX_PARTS: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum AttachmentError {
    #[error("attachment MIME could not be decoded")]
    Parse(#[from] mailparse::MailParseError),
    #[error("attachment inspection supports messages up to 64 MiB and 256 MIME parts")]
    Limit,
    #[error("attachment no longer exists in this message")]
    NotFound,
}

/// A suggested basename, never a sender-controlled path. Original spelling stays
/// in immutable MIME. Also used for local attachment display/header names.
pub fn attachment_name(name: &str) -> String {
    let name: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|ch| !ch.is_control() && *ch != ':')
        .take(180)
        .collect();
    let name = name.trim_matches(['.', ' ']);
    if name.is_empty() {
        "attachment.bin".into()
    } else {
        name.to_owned()
    }
}

fn metadata(part: &ParsedMail<'_>, id: &str) -> Option<Attachment> {
    let disposition = part.get_content_disposition();
    let name = disposition
        .params
        .get("filename")
        .or_else(|| part.ctype.params.get("name"));
    let explicit = disposition.disposition == DispositionType::Attachment;
    let binary = part.subparts.is_empty()
        && !matches!(part.ctype.mimetype.as_str(), "text/plain" | "text/html")
        && !part.ctype.mimetype.starts_with("multipart/");
    if !explicit && name.is_none() && !binary {
        return None;
    }
    Some(Attachment {
        id: id.to_owned(),
        name: attachment_name(name.map(String::as_str).unwrap_or(
            if part.ctype.mimetype == "message/rfc822" {
                "message.eml"
            } else {
                "attachment.bin"
            },
        )),
        media_type: part.ctype.mimetype.clone(),
        bytes: 0,
        inline: disposition.disposition == DispositionType::Inline && !explicit,
        content_id: part.headers.get_first_value("Content-ID").map(|id| {
            id.trim()
                .trim_start_matches('<')
                .trim_end_matches('>')
                .to_owned()
        }),
    })
}

fn visit(
    part: &ParsedMail<'_>,
    id: &str,
    count: &mut usize,
    visitor: &mut impl FnMut(&ParsedMail<'_>, Attachment) -> Result<(), AttachmentError>,
) -> Result<(), AttachmentError> {
    *count += 1;
    if *count > MAX_PARTS {
        return Err(AttachmentError::Limit);
    }
    if let Some(item) = metadata(part, id) {
        visitor(part, item)?;
    } else {
        for (index, child) in part.subparts.iter().enumerate() {
            visit(child, &format!("{id}.{index}"), count, visitor)?;
        }
    }
    Ok(())
}

fn parse(raw: &[u8]) -> Result<ParsedMail<'_>, AttachmentError> {
    if raw.len() > MAX_RAW {
        return Err(AttachmentError::Limit);
    }
    Ok(mailparse::parse_mail(raw)?)
}

impl MimeAttachments for MailparseParser {
    type Error = AttachmentError;

    fn attachments(&self, raw: &[u8]) -> Result<Vec<Attachment>, AttachmentError> {
        let parsed = parse(raw)?;
        let mut result = Vec::new();
        visit(&parsed, "0", &mut 0, &mut |part, mut item| {
            item.bytes = part.get_body_raw()?.len() as u64;
            result.push(item);
            Ok(())
        })?;
        Ok(result)
    }

    fn attachment(&self, raw: &[u8], id: &str) -> Result<Vec<u8>, AttachmentError> {
        let parsed = parse(raw)?;
        let mut result = None;
        visit(&parsed, "0", &mut 0, &mut |part, item| {
            if item.id == id {
                result = Some(part.get_body_raw()?);
            }
            Ok(())
        })?;
        result.ok_or(AttachmentError::NotFound)
    }
}
