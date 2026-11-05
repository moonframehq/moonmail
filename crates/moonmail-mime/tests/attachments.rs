use {
    moonmail_core::{MimeAttachments, MimeParser},
    moonmail_mime::{MailparseParser, StalwartParser, attachment_name},
};

type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
const RAW: &[u8] = include_bytes!("fixtures/attachments.eml");

#[test]
fn nested_inventory_preserves_binary_transfer_bytes_and_keeps_attached_messages_opaque() -> Result {
    let parser = MailparseParser;
    let files = parser.attachments(RAW)?;
    assert_eq!(files.len(), 4);
    assert_eq!(
        files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>(),
        ["sample.bin", "café.txt", "preview.png", "forwarded.eml"]
    );
    assert_eq!(files[0].id, "0.1");
    assert_eq!(
        parser.attachment(RAW, &files[0].id)?,
        [0, 1, 2, 3, 255, 254]
    );
    let text = parser.attachment(RAW, &files[1].id)?;
    assert!(
        text.starts_with(b"caf\xe9\r\n"),
        "save raw decoded bytes, not UTF-8 charset conversion"
    );
    assert!(files[2].inline);
    assert_eq!(parser.attachment(RAW, &files[2].id)?, b"\x89PNG\r\n\x1a\n");
    let message = parser.attachment(RAW, &files[3].id)?;
    assert!(message.starts_with(b"From: nested@example.org"));
    assert!(String::from_utf8_lossy(&message).contains("inner.bin"));
    for file in &files {
        assert_eq!(file.bytes, parser.attachment(RAW, &file.id)?.len() as u64);
    }
    assert!(
        parser.attachment(RAW, "0.0.0").is_err(),
        "outer body cannot be exported as an attachment"
    );
    assert!(parser.attachment(RAW, "../../file").is_err());
    for body in [
        parser.parse(RAW)?.text_body,
        StalwartParser::default().parse(RAW)?.text_body,
    ] {
        let body = body.ok_or("missing body")?;
        assert!(body.contains("synthetic files"));
        assert!(!body.contains("Inner payload"));
    }
    Ok(())
}

#[test]
fn root_unnamed_and_duplicate_files_have_stable_scoped_ids_and_limits_are_explicit() -> Result {
    let parser = MailparseParser;
    let raw =
        b"Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: base64\r\n\r\nAAEC";
    let root = parser.attachments(raw)?;
    assert_eq!(root[0].id, "0");
    assert_eq!(root[0].name, "attachment.bin");
    assert_eq!(parser.attachment(raw, "0")?, [0, 1, 2]);
    let mut raw = "Content-Type: multipart/mixed; boundary=x\r\n\r\n".to_owned();
    for _ in 0..2 {
        raw.push_str("--x\r\nContent-Type: text/plain; name=same.txt\r\n\r\nData\r\n");
    }
    raw.push_str("--x--\r\n");
    let files = parser.attachments(raw.as_bytes())?;
    assert_eq!(files.len(), 2);
    assert_eq!(files[0].name, files[1].name);
    assert_ne!(files[0].id, files[1].id);
    let raw = format!(
        "Content-Type: multipart/mixed; boundary=x\r\n\r\n{}--x--\r\n",
        "--x\r\nContent-Type: application/octet-stream\r\n\r\nx\r\n".repeat(256)
    );
    assert!(parser.attachments(raw.as_bytes()).is_err());
    assert!(
        parser
            .attachments(b"Content-Type: text/plain\r\n\r\nBody")?
            .is_empty()
    );
    assert_eq!(attachment_name("C:\\folder\\..\\file\r\n.txt"), "file.txt");
    assert_eq!(attachment_name("../.."), "attachment.bin");
    Ok(())
}
