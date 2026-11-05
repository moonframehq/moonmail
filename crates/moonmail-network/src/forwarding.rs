use {
    crate::{Result, recipients::editable, validate_address},
    moonmail_core::{MailDetail, ReplyHeaders},
    moonmail_store::Draft,
};

fn line(value: &str) -> String {
    value
        .chars()
        .map(|ch| if ch.is_control() { ' ' } else { ch })
        .collect()
}

/// Forwarding starts a new conversation with deliberately empty recipients and
/// no reply ancestry. Only public original headers enter the body; never Bcc.
/// The caller copies selected attachment bytes and supplies a fresh draft ID.
pub fn compose_forward(
    detail: &MailDetail,
    headers: &ReplyHeaders,
    account: &str,
) -> Result<Draft> {
    if !account.is_empty() {
        validate_address(account)?;
    }
    let original_subject = line(&detail.summary.subject);
    let subject = if original_subject.to_ascii_lowercase().starts_with("fwd:") {
        original_subject.clone()
    } else {
        format!("Fwd: {original_subject}")
    };
    let mut body = format!(
        "\n\n---------- Forwarded message ----------\nFrom: {}\nDate: {}\nSubject: {}\nTo: {}\n",
        line(&editable(&headers.addresses.from)),
        line(&detail.summary.date_label),
        original_subject,
        line(&editable(&headers.addresses.to))
    );
    if !headers.addresses.cc.is_empty() {
        body.push_str(&format!("Cc: {}\n", line(&editable(&headers.addresses.cc))));
    }
    body.push('\n');
    let quote: String = detail.text.chars().take(64_000).collect();
    body.push_str(&quote);
    if quote.len() < detail.text.len() {
        body.push_str("\n[Forwarded text truncated]\n");
    }
    Ok(Draft {
        account: account.into(),
        subject,
        body,
        ..Default::default()
    })
}
