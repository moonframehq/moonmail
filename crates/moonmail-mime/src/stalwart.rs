use {
    crate::normalize::normalize_bodies,
    mail_parser::{AddressList, MessageParser, PartKind},
    moonmail_core::{MailboxAddress, MimeParser, ParsedMessage},
};

#[derive(Debug, thiserror::Error)]
pub enum StalwartError {
    #[error("message could not be parsed")]
    InvalidMessage,
}

#[derive(Default)]
pub struct StalwartParser {
    parser: MessageParser,
}

impl MimeParser for StalwartParser {
    type Error = StalwartError;

    fn parse(&self, raw: &[u8]) -> Result<ParsedMessage, Self::Error> {
        let message = self
            .parser
            .parse(raw)
            .ok_or(StalwartError::InvalidMessage)?;
        Ok(normalize_bodies(ParsedMessage {
            date: message.date().map(|date| date.to_timestamp()),
            message_id: message.message_id().map(str::to_owned),
            references: message
                .references()
                .into_iter()
                .flat_map(|ids| ids.iter())
                .map(str::to_owned)
                .collect(),
            in_reply_to: message
                .in_reply_to()
                .and_then(|ids| ids.iter().next())
                .map(str::to_owned),
            subject: message.subject().map(str::to_owned),
            from: addresses(message.from()),
            reply_to: addresses(message.reply_to()),
            to: addresses(message.to()),
            cc: addresses(message.cc()),
            text_body: message
                .text_body()
                .find(|part| matches!(part.kind(), PartKind::Text))
                .and_then(|part| part.text())
                .map(|text| text.into_owned()),
            html_body: message
                .html_body()
                .find(|part| matches!(part.kind(), PartKind::Html))
                .and_then(|part| part.text())
                .map(|text| text.into_owned()),
        }))
    }
}

fn addresses(list: Option<AddressList<'_>>) -> Vec<MailboxAddress> {
    list.into_iter()
        .flat_map(|list| list.mailboxes())
        .filter_map(|mailbox| {
            Some(MailboxAddress {
                name: mailbox.name().map(str::to_owned),
                address: mailbox.address()?.to_owned(),
            })
        })
        .collect()
}
