use {
    crate::{Error, RemoteFolder, Result},
    imap_proto::{MailboxDatum, NameAttribute, Response},
    moonmail_core::MailboxRole,
};

/// Gmail advertises its archive as \All, including localized folder names.
/// Keep the mapping provider-scoped; other servers' All folders are virtual views.
pub(crate) fn provider_roles(provider: crate::Provider, folders: &mut [RemoteFolder]) {
    if provider == crate::Provider::Google {
        for folder in folders {
            if folder.role == Some(MailboxRole::All) {
                folder.role = Some(MailboxRole::Archive);
            }
        }
    }
}

pub(crate) fn role(name: &str, attributes: &[NameAttribute<'_>]) -> Option<MailboxRole> {
    if name.eq_ignore_ascii_case("INBOX") {
        return Some(MailboxRole::Inbox);
    }
    let mut roles = attributes.iter().filter_map(|attribute| {
        Some(match attribute {
            NameAttribute::Archive => MailboxRole::Archive,
            NameAttribute::Trash => MailboxRole::Trash,
            NameAttribute::Sent => MailboxRole::Sent,
            NameAttribute::Drafts => MailboxRole::Drafts,
            NameAttribute::Junk => MailboxRole::Junk,
            NameAttribute::All => MailboxRole::All,
            NameAttribute::Flagged => MailboxRole::Flagged,
            _ => return None,
        })
    });
    let first = roles.next()?;
    // Conflicting roles need a user override, not an arbitrary target choice.
    roles.all(|role| role == first).then_some(first)
}

/// Called only after the adapter has received the final tagged OK. Parse every
/// response or fail the whole list; do not turn malformed tails into retirement.
pub(crate) fn parse_list(mut input: &[u8]) -> Result<Vec<RemoteFolder>> {
    let mut folders = Vec::new();
    while !input.is_empty() {
        let (rest, response) = imap_proto::parser::parse_response(input)
            .map_err(|_| Error::Invalid("Malformed complete LIST response"))?;
        if rest.len() >= input.len() {
            return Err(Error::Invalid("LIST parser made no progress"));
        }
        if let Response::MailboxData(MailboxDatum::List {
            name_attributes,
            delimiter,
            name,
        }) = response
        {
            folders.push(RemoteFolder {
                role: role(&name, &name_attributes),
                selectable: !name_attributes.contains(&NameAttribute::NoSelect),
                delimiter: delimiter.map(|value| value.into_owned()),
                name: name.into_owned(),
            });
        }
        input = rest;
    }
    Ok(folders)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gmail_all_mail_is_the_archive_but_other_providers_keep_all_separate() -> Result<()> {
        let raw = b"* LIST (\\All) \"/\" \"[Gmail]/Tous les messages\"\r\n* LIST (\\Sent) \"/\" \"[Gmail]/Messages envoyes\"\r\n";
        let mut folders = parse_list(raw)?;
        provider_roles(crate::Provider::Other, &mut folders);
        assert_eq!(folders[0].role, Some(MailboxRole::All));
        provider_roles(crate::Provider::Google, &mut folders);
        assert_eq!(folders[0].role, Some(MailboxRole::Archive));
        assert_eq!(folders[1].role, Some(MailboxRole::Sent));
        Ok(())
    }
    #[test]
    fn special_use_handles_localized_folders_literals_and_malformed_tails() -> Result<()> {
        let folders = parse_list(b"* LIST (\\Archive) \"/\" \"Conserves\"\r\n* LIST (\\Trash) \".\" {9}\r\nCorbeille\r\n* LIST (\\Noselect) \"/\" \"Parent\"\r\n* LIST () NIL \"INBOX\"\r\n")?;
        assert_eq!(folders[0].role, Some(MailboxRole::Archive));
        assert_eq!(folders[1].role, Some(MailboxRole::Trash));
        assert_eq!(folders[1].name, "Corbeille");
        assert!(!folders[2].selectable);
        assert_eq!(folders[3].role, Some(MailboxRole::Inbox));
        assert!(parse_list(b"* LIST (\\Archive) NIL \"Old\"\r\n* LIST (").is_err());
        assert_eq!(
            role("Trash", &[]),
            None,
            "display names never choose a destructive target"
        );
        assert_eq!(
            role(
                "Conflicting",
                &[NameAttribute::Archive, NameAttribute::Trash]
            ),
            None
        );
        Ok(())
    }
}
