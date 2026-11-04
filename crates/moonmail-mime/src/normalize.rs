use moonmail_core::ParsedMessage;

/// Transfer decoders disagree on newline preservation. Keep the extracted view
/// stable; original message bytes remain the responsibility of the raw store.
pub(crate) fn normalize_bodies(mut message: ParsedMessage) -> ParsedMessage {
    for body in [&mut message.text_body, &mut message.html_body]
        .into_iter()
        .flatten()
    {
        if body.contains("\r\n") {
            *body = body.replace("\r\n", "\n");
        }
    }
    message
}
