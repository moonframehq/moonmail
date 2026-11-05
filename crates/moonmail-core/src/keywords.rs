/// Canonical IMAP keyword: ASCII case-insensitive atom, at most 255 bytes.
/// System flags, wildcards, quoting, controls and non-ASCII are not keywords.
/// Friendly Unicode names belong in account-local labels, never on the wire.
pub fn canonical_keyword(value: &str) -> Option<String> {
    if value.is_empty()
        || value.len() > 255
        || value
            .bytes()
            .any(|b| !(0x21..=0x7e).contains(&b) || b"(){%*\"\\]".contains(&b))
    {
        None
    } else {
        Some(value.to_ascii_lowercase())
    }
}
