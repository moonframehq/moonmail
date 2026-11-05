use {
    crate::{Result, recipients::address_key, validate_address},
    moonmail_core::ReplyHeaders,
    moonmail_store::{Draft, Identity},
};

pub fn validate_identity(identity: &Identity) -> Result<()> {
    identity.validate()?;
    validate_address(&identity.account)?;
    validate_address(&identity.address)?;
    if !identity.reply_to.is_empty() {
        validate_address(&identity.reply_to)?;
    }
    Ok(())
}

/// Prefer the original From for one's own sent message, otherwise the first
/// configured identity addressed in To then Cc; fall back to an explicit default.
/// Never infer aliases by plus/dot stripping or local-part case folding.
pub fn choose_identity(
    identities: &[Identity],
    account: &str,
    headers: Option<&ReplyHeaders>,
) -> Option<Identity> {
    let find = |address: &str| {
        identities
            .iter()
            .find(|identity| {
                identity.account == account
                    && address_key(&identity.address) == address_key(address)
            })
            .cloned()
    };
    if let Some(headers) = headers {
        let from = &headers.addresses.from;
        if !from.is_empty()
            && from.iter().all(|address| {
                address_key(&address.address) == address_key(account)
                    || find(&address.address).is_some()
            })
            && let Some(identity) = from.iter().find_map(|address| find(&address.address))
        {
            return Some(identity);
        }
        if let Some(identity) = headers
            .addresses
            .to
            .iter()
            .chain(&headers.addresses.cc)
            .find_map(|address| find(&address.address))
        {
            return Some(identity);
        }
    }
    identities
        .iter()
        .find(|identity| identity.account == account && identity.is_default)
        .cloned()
}

/// Explicit insertion never removes or rewrites an existing user's signature.
/// New drafts call this once; reopened drafts retain their exact saved body.
pub fn insert_signature(draft: &mut Draft) {
    if let Some(identity) = &draft.identity
        && !identity.signature.is_empty()
    {
        draft.body = format!("\n\n-- \n{}\n{}", identity.signature, draft.body);
    }
}
