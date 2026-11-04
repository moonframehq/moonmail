use moonmail_core::{MessageId, SearchDocument};

pub fn mime_fixtures() -> [(&'static str, &'static [u8]); 4] {
    [
        ("plain", include_bytes!("../../../fixtures/mime/plain.eml")),
        (
            "encoded",
            include_bytes!("../../../fixtures/mime/encoded.eml"),
        ),
        ("html", include_bytes!("../../../fixtures/mime/html.eml")),
        (
            "multipart",
            include_bytes!("../../../fixtures/mime/multipart.eml"),
        ),
    ]
}

pub fn plain_message(repetitions: usize) -> Vec<u8> {
    format!(
        "From: Alice <alice@example.com>\r\nTo: Bob <bob@example.org>\r\n\
         Subject: Synthetic correspondence\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{}",
        "A synthetic message about the quarterly project review.\r\n".repeat(repetitions),
    )
    .into_bytes()
}

pub fn documents(count: u64) -> Vec<(MessageId, SearchDocument)> {
    (0..count)
        .map(|id| {
            let marker = if id % 997 == 0 {
                "uniquecheckpoint"
            } else {
                "ordinary"
            };
            (
                MessageId(id),
                SearchDocument {
                    subject: format!("Project review {id}"),
                    body: format!(
                        "{marker} {}",
                        "quarterly project correspondence ".repeat(32)
                    ),
                    from: "Alice alice@example.com".into(),
                    to: "Bob bob@example.org".into(),
                    cc: String::new(),
                },
            )
        })
        .collect()
}
