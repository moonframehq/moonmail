use {
    crate::{
        Error, Result,
        recipients::{address_key, editable, unique},
        validate_address,
    },
    moonmail_core::{MailDetail, MailboxAddress, ReplyHeaders},
    moonmail_store::Draft,
    std::collections::HashSet,
};

#[derive(Clone, Copy)]
pub enum ReplyMode {
    Sender,
    All,
}

/// Construct an unsaved draft. Prefer Reply-To over From. Reply-All keeps original
/// To recipients in To and Cc recipients in Cc, subtracting the known account and
/// deduplicating across roles. A reply to one's own sent message targets original
/// recipients. Bcc is never recovered or copied. The caller supplies a fresh ID.
pub fn compose_reply(
    detail: &MailDetail,
    headers: ReplyHeaders,
    account: &str,
    mode: ReplyMode,
) -> Result<Draft> {
    compose_reply_with_identities(detail, headers, account, mode, &[])
}

pub fn compose_reply_with_identities(
    detail: &MailDetail,
    headers: ReplyHeaders,
    account: &str,
    mode: ReplyMode,
    identities: &[moonmail_store::Identity],
) -> Result<Draft> {
    let addresses = &headers.addresses;
    let preferred = if addresses.reply_to.is_empty() {
        &addresses.from
    } else {
        &addresses.reply_to
    };
    let mut seen = HashSet::new();
    if !account.is_empty() {
        validate_address(account)?;
        seen.insert(address_key(account));
    }
    for identity in identities
        .iter()
        .filter(|identity| identity.account == account)
    {
        crate::validate_identity(identity)?;
        seen.insert(address_key(&identity.address));
    }
    let owned = seen.clone();
    let validate = |values: &[MailboxAddress]| -> Result<()> {
        for value in values {
            validate_address(&value.address)?;
        }
        Ok(())
    };
    // Replying to one's own sent mail should reach its original recipients.
    let sent_by_self = !addresses.from.is_empty()
        && !account.is_empty()
        && addresses
            .from
            .iter()
            .all(|address| owned.contains(&address_key(&address.address)));
    let preferred = if sent_by_self {
        &addresses.to
    } else {
        preferred
    };
    validate(preferred)?;
    let mut to = unique(preferred.clone(), &mut seen);
    if matches!(mode, ReplyMode::All) && !sent_by_self {
        validate(&addresses.to)?;
        to.extend(unique(addresses.to.clone(), &mut seen));
    }
    let cc = if matches!(mode, ReplyMode::All) {
        validate(&addresses.cc)?;
        unique(addresses.cc.clone(), &mut seen)
    } else {
        Vec::new()
    };
    if to.is_empty() && cc.is_empty() {
        return Err(Error::Invalid(
            "No reply recipients remain after excluding your account",
        ));
    }
    if to.len() + cc.len() > 100 {
        return Err(Error::Invalid(
            "Reply has more than 100 recipients; compose a smaller recipient list",
        ));
    }
    let mut references = headers.references;
    if references.is_empty() {
        references.extend(headers.in_reply_to);
    }
    let subject = if detail
        .summary
        .subject
        .to_ascii_lowercase()
        .starts_with("re:")
    {
        detail.summary.subject.clone()
    } else {
        format!("Re: {}", detail.summary.subject)
    };
    // Bound UI/draft growth for huge messages and visibly mark truncation.
    let quote: String = detail.text.chars().take(64_000).collect();
    let mut body = format!(
        "\n\nOn {}, {} wrote:\n",
        detail.summary.date_label,
        editable(&addresses.from)
    );
    for line in quote.lines() {
        body.push_str("> ");
        body.push_str(line);
        body.push('\n');
    }
    if quote.len() < detail.text.len() {
        body.push_str("> [Quoted text truncated]\n");
    }
    Ok(Draft {
        recipients: editable(&to),
        cc: editable(&cc),
        account: account.to_owned(),
        subject,
        body,
        reply_id: headers.message_id.unwrap_or_default(),
        references,
        ..Default::default()
    })
}
