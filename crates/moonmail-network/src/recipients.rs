use {
    crate::{Error, Result, validate_address},
    moonmail_core::MailboxAddress,
    std::collections::HashSet,
};

/// Explicit identity comparison: preserve local-part spelling/case; fold ASCII
/// domain case only. No plus stripping, provider alias guessing, or dot folding.
pub(crate) fn address_key(address: &str) -> String {
    match address.rsplit_once('@') {
        Some((local, domain)) => format!("{local}@{}", domain.to_ascii_lowercase()),
        None => address.to_owned(),
    }
}

pub(crate) fn parse_recipients(value: &str) -> Result<Vec<MailboxAddress>> {
    if value.len() > 64 * 1024 || value.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "Recipient fields must be single lines of at most 64 KiB",
        ));
    }
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let parsed = mailparse::addrparse(value)?;
    let mut addresses = Vec::new();
    for entry in parsed.iter() {
        let entries = match entry {
            mailparse::MailAddr::Single(single) => std::slice::from_ref(single),
            mailparse::MailAddr::Group(group) => group.addrs.as_slice(),
        };
        for single in entries {
            validate_address(&single.addr)?;
            if single
                .display_name
                .as_deref()
                .is_some_and(|name| name.chars().any(char::is_control))
            {
                return Err(Error::Invalid(
                    "Recipient display names cannot contain control characters",
                ));
            }
            addresses.push(MailboxAddress {
                name: single.display_name.clone(),
                address: single.addr.clone(),
            });
            if addresses.len() > 100 {
                return Err(Error::Invalid("Enter at most 100 recipients per field"));
            }
        }
    }
    Ok(addresses)
}

pub(crate) fn unique(
    addresses: Vec<MailboxAddress>,
    seen: &mut HashSet<String>,
) -> Vec<MailboxAddress> {
    addresses
        .into_iter()
        .filter(|address| seen.insert(address_key(&address.address)))
        .collect()
}

pub(crate) fn header_addresses(
    addresses: &[MailboxAddress],
) -> Vec<mail_builder::headers::address::Address<'_>> {
    addresses
        .iter()
        .map(|address| {
            mail_builder::headers::address::Address::new_address(
                address.name.as_deref(),
                address.address.as_str(),
            )
        })
        .collect()
}

/// Editable recipient text. Quote display-name commas/quotes and remove decoded
/// header controls so it can be safely round-tripped through the compose fields.
pub(crate) fn editable(addresses: &[MailboxAddress]) -> String {
    addresses
        .iter()
        .map(
            |address| match address.name.as_deref().filter(|name| !name.is_empty()) {
                Some(name) => {
                    let name: String = name
                        .chars()
                        .map(|ch| if ch.is_control() { ' ' } else { ch })
                        .collect();
                    format!(
                        "\"{}\" <{}>",
                        name.replace('\\', "\\\\").replace('"', "\\\""),
                        address.address
                    )
                },
                None => address.address.clone(),
            },
        )
        .collect::<Vec<_>>()
        .join(", ")
}
