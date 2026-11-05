use {
    crate::{
        Error, Result,
        server::Shared,
        store::{ACCOUNTS, PASSWORD},
        wire::Wire,
    },
    imap_codec::{
        CommandCodec,
        decode::{CommandDecodeError, Decoder},
        imap_types::{
            command::{Command, CommandBody},
            core::LiteralMode,
            fetch::{MacroOrMessageDataItemNames, MessageDataItemName},
            flag::{StoreResponse, StoreType},
            mailbox::Mailbox,
            sequence::{SeqOrUid, Sequence, SequenceSet},
        },
    },
    std::{collections::BTreeSet, io::Write, sync::Arc},
};

#[derive(Default)]
struct Session {
    account: Option<String>,
    selected: Option<String>,
    readonly: bool,
}

pub(crate) fn serve(wire: &mut Wire, shared: &Shared) -> Result {
    wire.send("* OK Moonmail ephemeral test IMAP\r\n")?;
    let mut session = Session::default();
    loop {
        let command = command(wire)?;
        let tag = command.tag.as_ref();
        match &command.body {
            CommandBody::Capability => {
                wire.send(if wire.encrypted {
                    "* CAPABILITY IMAP4rev1 UIDPLUS MOVE\r\n"
                } else {
                    "* CAPABILITY IMAP4rev1 STARTTLS LOGINDISABLED\r\n"
                })?;
                wire.send(format!("{tag} OK capabilities\r\n"))?;
            },
            CommandBody::Logout => {
                wire.send(format!("* BYE closing\r\n{tag} OK logout\r\n"))?;
                return Ok(());
            },
            CommandBody::Noop => wire.send(format!("{tag} OK noop\r\n"))?,
            CommandBody::StartTLS if !wire.encrypted && session.account.is_none() => {
                wire.send(format!("{tag} OK begin TLS\r\n"))?;
                wire.upgrade(Arc::clone(&shared.tls))?;
            },
            CommandBody::Login { username, password }
                if wire.encrypted && session.account.is_none() =>
            {
                let user = std::str::from_utf8(username.as_ref()).unwrap_or_default();
                if ACCOUNTS.contains(&user) && password.declassify().as_ref() == PASSWORD.as_bytes()
                {
                    session.account = Some(user.into());
                    wire.send(format!("{tag} OK authenticated\r\n"))?;
                } else {
                    wire.send(format!("{tag} NO authentication failed\r\n"))?;
                }
            },
            _ if session.account.is_none() => {
                wire.send(format!("{tag} NO TLS and login required\r\n"))?
            },
            _ => match execute(&command, &mut session, shared) {
                Ok(response) => wire.send(response)?,
                Err(Error::Invalid(reason)) => wire.send(format!("{tag} NO {reason}\r\n"))?,
                Err(error) => return Err(error),
            },
        }
    }
}

fn command(wire: &mut Wire) -> Result<Command<'static>> {
    let mut data = wire.line(8192)?;
    for _ in 0..16 {
        match CommandCodec::default().decode_static(&data) {
            Ok(([], command)) => return Ok(command),
            Err(CommandDecodeError::LiteralFound { length, mode, .. }) => {
                if data.len() + length as usize > crate::store::MAX_MESSAGE + 8192 {
                    return Err(Error::Invalid("Command too large"));
                }
                if mode != LiteralMode::Sync {
                    return Err(Error::Invalid(
                        "Non-synchronizing literals are not advertised",
                    ));
                }
                wire.send("+ send literal\r\n")?;
                data.extend(wire.literal(length as usize)?);
                data.extend(wire.line(8192)?);
            },
            _ => return Err(Error::Invalid("Malformed or unsupported IMAP command")),
        }
    }
    Err(Error::Invalid("Too many command literals"))
}

fn name(mailbox: &Mailbox<'_>) -> Result<String> {
    match mailbox {
        Mailbox::Inbox => Ok("INBOX".into()),
        Mailbox::Other(other) => std::str::from_utf8(other.as_ref())
            .map(str::to_owned)
            .map_err(|_| Error::Invalid("Fixture folder must be UTF-8")),
    }
}

fn matches(set: &SequenceSet, value: u32, largest: u32) -> bool {
    let number = |n: &SeqOrUid| match n {
        SeqOrUid::Value(n) => n.get(),
        SeqOrUid::Asterisk => largest,
    };
    set.0.as_ref().iter().any(|s| match s {
        Sequence::Single(n) => number(n) == value,
        Sequence::Range(a, b) => {
            (number(a).min(number(b))..=number(a).max(number(b))).contains(&value)
        },
    })
}

fn execute(command: &Command<'_>, session: &mut Session, shared: &Shared) -> Result<Vec<u8>> {
    let account = session
        .account
        .as_deref()
        .ok_or(Error::Invalid("Login required"))?;
    let tag = command.tag.as_ref();
    let mut store = shared.store()?;
    let mut out = Vec::new();
    let mut code = String::new();
    match &command.body {
        CommandBody::Check if session.selected.is_some() => {},
        CommandBody::List {
            reference,
            mailbox_wildcard,
        } => {
            use imap_codec::imap_types::mailbox::ListMailbox;
            let pattern = match mailbox_wildcard {
                ListMailbox::Token(t) => t.as_ref(),
                ListMailbox::String(s) => s.as_ref(),
            };
            if !name(reference)?.is_empty() || !matches!(pattern, b"*" | b"%") {
                return Err(Error::Invalid(
                    "Fixture LIST supports only an empty reference and * or %",
                ));
            }
            for (folder, role) in [
                ("INBOX", ""),
                ("Sent", "\\Sent"),
                ("Archive", "\\Archive"),
                ("Trash", "\\Trash"),
                ("Junk", "\\Junk"),
                ("Drafts", "\\Drafts"),
            ] {
                write!(out, "* LIST ({role}) \"/\" \"{folder}\"\r\n")?;
            }
        },
        CommandBody::Select { mailbox, .. } | CommandBody::Examine { mailbox, .. } => {
            session.selected = None;
            let folder = name(mailbox)?;
            let selected = store.folder(account, &folder)?;
            session.readonly = matches!(command.body, CommandBody::Examine { .. });
            write!(
                out,
                "* FLAGS (\\Seen \\Flagged \\Deleted \\Answered \\Draft)\r\n* {} EXISTS\r\n* 0 RECENT\r\n* OK [UIDVALIDITY {}] epoch\r\n* OK [UIDNEXT {}] next\r\n* OK [PERMANENTFLAGS (\\Seen \\Flagged \\Deleted \\Answered \\Draft \\*)] flags\r\n",
                selected.messages.len(),
                store.epoch,
                selected.next
            )?;
            code = if session.readonly {
                "[READ-ONLY] "
            } else {
                "[READ-WRITE] "
            }
            .into();
            session.selected = Some(folder);
        },
        CommandBody::Append {
            mailbox,
            flags,
            message,
            date: None,
        } => {
            let folder = name(mailbox)?;
            let imap_codec::imap_types::extensions::binary::LiteralOrLiteral8::Literal(message) =
                message
            else {
                return Err(Error::Invalid("BINARY is not supported"));
            };
            let uid = store.append(
                account,
                &folder,
                message.as_ref(),
                flags.iter().map(ToString::to_string).collect(),
            )?;
            code = format!("[APPENDUID {} {uid}] ", store.epoch);
        },
        CommandBody::Fetch {
            sequence_set,
            macro_or_item_names: MacroOrMessageDataItemNames::MessageDataItemNames(items),
            uid: true,
            ..
        } => {
            if items.len() > 16
                || items
                    .iter()
                    .filter(|item| matches!(item, MessageDataItemName::BodyExt { .. }))
                    .count()
                    > 1
                || items.iter().any(|item| {
                    !matches!(
                        item,
                        MessageDataItemName::Uid
                            | MessageDataItemName::Flags
                            | MessageDataItemName::Rfc822Size
                            | MessageDataItemName::InternalDate
                            | MessageDataItemName::BodyExt {
                                section: None,
                                partial: None,
                                ..
                            }
                    )
                })
            {
                return Err(Error::Invalid("Unsupported FETCH item"));
            }
            let folder = store.folder_mut(
                account,
                session
                    .selected
                    .as_deref()
                    .ok_or(Error::Invalid("Select a mailbox"))?,
            )?;
            let largest = folder.messages.last().map_or(0, |m| m.uid);
            for (index, message) in folder
                .messages
                .iter_mut()
                .enumerate()
                .filter(|(_, m)| matches(sequence_set, m.uid, largest))
            {
                write!(out, "* {} FETCH (UID {}", index + 1, message.uid)?;
                for item in items {
                    match item {
                        MessageDataItemName::Uid => {},
                        MessageDataItemName::Flags => write!(
                            out,
                            " FLAGS ({})",
                            message.flags.iter().cloned().collect::<Vec<_>>().join(" ")
                        )?,
                        MessageDataItemName::Rfc822Size => {
                            write!(out, " RFC822.SIZE {}", message.raw.len())?
                        },
                        MessageDataItemName::InternalDate => {
                            out.extend_from_slice(b" INTERNALDATE \"01-Jan-2026 00:00:00 +0000\"")
                        },
                        MessageDataItemName::BodyExt {
                            section: None,
                            partial: None,
                            peek,
                        } => {
                            if !peek && !session.readonly {
                                message.flags.insert("\\Seen".into());
                            }
                            write!(out, " BODY[] {{{}}}\r\n", message.raw.len())?;
                            out.extend_from_slice(&message.raw);
                        },
                        _ => return Err(Error::Invalid("Unsupported FETCH item")),
                    }
                }
                out.extend_from_slice(b")\r\n");
            }
        },
        CommandBody::Store {
            sequence_set,
            kind,
            response,
            flags,
            uid: true,
            ..
        } if !session.readonly => {
            let folder = store.folder_mut(
                account,
                session
                    .selected
                    .as_deref()
                    .ok_or(Error::Invalid("Select a mailbox"))?,
            )?;
            let largest = folder.messages.last().map_or(0, |m| m.uid);
            let flags: BTreeSet<String> = flags.iter().map(ToString::to_string).collect();
            // Stage every update so a quota rejection cannot partially change a UID set.
            let mut updates = Vec::new();
            for (index, message) in folder
                .messages
                .iter()
                .enumerate()
                .filter(|(_, m)| matches(sequence_set, m.uid, largest))
            {
                let mut updated = message.flags.clone();
                match kind {
                    StoreType::Replace => updated = flags.clone(),
                    StoreType::Add => {
                        for flag in &flags {
                            if !updated.iter().any(|f| f.eq_ignore_ascii_case(flag)) {
                                updated.insert(flag.clone());
                            }
                        }
                    },
                    StoreType::Remove => {
                        updated.retain(|flag| !flags.iter().any(|f| f.eq_ignore_ascii_case(flag)))
                    },
                }
                crate::store::validate_flags(&updated)?;
                updates.push((index, updated));
            }
            for (index, flags) in updates {
                let message = &mut folder.messages[index];
                message.flags = flags;
                if *response == StoreResponse::Answer {
                    write!(
                        out,
                        "* {} FETCH (UID {} FLAGS ({}))\r\n",
                        index + 1,
                        message.uid,
                        message.flags.iter().cloned().collect::<Vec<_>>().join(" ")
                    )?;
                }
            }
        },
        CommandBody::Copy {
            sequence_set,
            mailbox,
            uid: true,
        }
        | CommandBody::Move {
            sequence_set,
            mailbox,
            uid: true,
        } if !session.readonly => {
            let source = session
                .selected
                .as_deref()
                .ok_or(Error::Invalid("Select a mailbox"))?;
            let folder = store.folder(account, source)?;
            let largest = folder.messages.last().map_or(0, |m| m.uid);
            let messages: Vec<_> = folder
                .messages
                .iter()
                .enumerate()
                .filter(|(_, m)| matches(sequence_set, m.uid, largest))
                .map(|(i, m)| (i, m.clone()))
                .collect();
            if messages.len() != 1 {
                return Err(Error::Invalid("Fixture COPY/MOVE supports exactly one UID"));
            }
            let (index, message) = &messages[0];
            let destination = name(mailbox)?;
            if destination == source {
                return Err(Error::Invalid("Destination equals source"));
            }
            let uid = store.append(account, &destination, &message.raw, message.flags.clone())?;
            code = format!("[COPYUID {} {} {uid}] ", store.epoch, message.uid);
            if matches!(command.body, CommandBody::Move { .. }) {
                store.remove(account, source, *index)?;
                write!(out, "* {} EXPUNGE\r\n", index + 1)?;
            }
        },
        CommandBody::ExpungeUid { sequence_set } if !session.readonly => {
            let source = session
                .selected
                .as_deref()
                .ok_or(Error::Invalid("Select a mailbox"))?;
            let folder = store.folder(account, source)?;
            let largest = folder.messages.last().map_or(0, |m| m.uid);
            let mut removed = Vec::new();
            for index in (0..folder.messages.len()).rev() {
                let message = &folder.messages[index];
                if message.flags.contains("\\Deleted")
                    && matches(sequence_set, message.uid, largest)
                {
                    removed.push(index);
                }
            }
            for index in removed {
                store.remove(account, source, index)?;
                write!(out, "* {} EXPUNGE\r\n", index + 1)?;
            }
        },
        _ => return Err(Error::Invalid("Unsupported command in this fixture daemon")),
    }
    write!(out, "{tag} OK {code}completed\r\n")?;
    Ok(out)
}
