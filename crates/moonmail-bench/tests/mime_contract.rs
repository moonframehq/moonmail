use {
    moonmail_bench::mime_fixtures,
    moonmail_core::MimeParser,
    moonmail_mime::{MailparseParser, StalwartParser},
    std::error::Error,
};

type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

fn extraction_contract<P: MimeParser>(parser: P) -> Result {
    let fixtures = mime_fixtures();
    let plain = parser.parse(fixtures[0].1)?;
    assert_eq!(plain.subject.as_deref(), Some("Quarterly review"));
    assert_eq!(plain.from[0].name.as_deref(), Some("Alice"));
    assert_eq!(plain.from[0].address, "alice@example.com");
    assert_eq!(
        plain
            .to
            .iter()
            .map(|mailbox| mailbox.address.as_str())
            .collect::<Vec<_>>(),
        ["bob@example.org", "carol@example.net"]
    );
    assert_eq!(plain.cc[0].address, "dave@example.com");
    assert_eq!(
        plain.text_body.as_deref().map(str::trim),
        Some("Discuss the quarterly review.")
    );
    assert_eq!(plain.html_body, None);

    let encoded = parser.parse(fixtures[1].1)?;
    assert_eq!(encoded.subject.as_deref(), Some("Café review"));
    assert_eq!(encoded.from[0].name.as_deref(), Some("André"));
    assert_eq!(
        encoded.text_body.as_deref().map(str::trim),
        Some("Café review.")
    );

    let html = parser.parse(fixtures[2].1)?;
    assert_eq!(
        html.text_body, None,
        "do not synthesize missing MIME alternatives"
    );
    assert_eq!(
        html.html_body.as_deref().map(str::trim),
        Some("<p>Actual HTML body.</p>")
    );

    let multipart = parser.parse(fixtures[3].1)?;
    assert_eq!(multipart.text_body.as_deref(), Some("Plain alternative."));
    assert_eq!(
        multipart.html_body.as_deref().map(str::trim),
        Some("<p>HTML alternative.</p>")
    );
    Ok(())
}

#[test]
fn stalwart_extraction_contract() -> Result {
    extraction_contract(StalwartParser::default())
}

#[test]
fn mailparse_extraction_contract() -> Result {
    extraction_contract(MailparseParser)
}

#[test]
fn adapters_agree_on_benchmark_corpus() -> Result {
    let stalwart = StalwartParser::default();
    for (name, bytes) in mime_fixtures() {
        assert_eq!(
            stalwart.parse(bytes)?,
            MailparseParser.parse(bytes)?,
            "fixture: {name}"
        );
    }
    Ok(())
}
