use {
    anyhow::{Context, Result, anyhow},
    base64::{Engine, engine::general_purpose::STANDARD},
    moonmail_core::MimeAttachments,
    serde::Serialize,
    std::{
        collections::{HashMap, HashSet},
        io::Cursor,
        sync::{Arc, Mutex},
    },
};

#[derive(Clone, Default, Serialize)]
pub struct ReaderLinks {
    pub items: Vec<String>,
    pub truncated: bool,
}

/// Only explicit HTTP(S) URLs are offered to the system browser. Relative URLs,
/// credentials, custom schemes, and controls are not external reader actions.
pub fn external_url(value: &str) -> Option<String> {
    if value.len() > 8192 || value.chars().any(char::is_control) {
        return None;
    }
    let url = url::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    Some(url.into())
}

fn push_link(links: &mut ReaderLinks, value: &str) {
    if let Some(url) = external_url(value) {
        if links.items.contains(&url) {
            return;
        }
        if links.items.len() == 100 {
            links.truncated = true;
        } else {
            links.items.push(url);
        }
    }
}

pub fn links(text: &str, html: Option<&str>) -> Result<ReaderLinks> {
    const MAX_SCAN: usize = 512 * 1024;
    let result = Arc::new(Mutex::new(ReaderLinks::default()));
    if let Some(html) = html {
        let collected = result.clone();
        let end = html.floor_char_boundary(MAX_SCAN.min(html.len()));
        ammonia::Builder::default()
            .url_schemes(HashSet::from(["http", "https"]))
            .url_relative(ammonia::UrlRelative::Deny)
            .attribute_filter(move |element, attribute, value| {
                if element == "a"
                    && attribute == "href"
                    && let Ok(mut links) = collected.lock()
                {
                    push_link(&mut links, value);
                }
                Some(value.into())
            })
            .clean(&html[..end]);
        result
            .lock()
            .map_err(|_| anyhow!("Reader link collection failed"))?
            .truncated |= end < html.len();
    }
    let end = text.floor_char_boundary(MAX_SCAN.min(text.len()));
    let mut links = result
        .lock()
        .map_err(|_| anyhow!("Reader link collection failed"))?;
    let mut finder = linkify::LinkFinder::new();
    finder.kinds(&[linkify::LinkKind::Url]);
    for link in finder.links(&text[..end]) {
        push_link(&mut links, link.as_str());
    }
    links.truncated |= end < text.len();
    Ok(links.clone())
}

#[derive(Default)]
pub struct EmbeddedImages {
    pub resources: HashMap<String, String>,
    pub skipped: usize,
}

/// Render only local CID raster images after bounded decoding and re-encoding.
/// Converting to PNG removes active/vector content, metadata and animation.
/// No URL from the message is fetched, and duplicate CIDs are never guessed.
pub fn images<P: MimeAttachments>(raw: &[u8], parser: &P) -> Result<EmbeddedImages> {
    let parts = parser.attachments(raw)?;
    let mut images = EmbeddedImages::default();
    let mut seen = HashSet::new();
    let mut total = 0usize;
    for part in parts {
        let Some(cid) = part.content_id else {
            continue;
        };
        if !seen.insert(cid.clone()) {
            images.resources.remove(&cid);
            images.skipped += 1;
            continue;
        }
        if cid.is_empty()
            || cid.len() > 998
            || cid.chars().any(char::is_control)
            || part.bytes > 2 * 1024 * 1024
            || images.resources.len() >= 16
        {
            images.skipped += 1;
            continue;
        }
        let bytes = parser.attachment(raw, &part.id)?;
        match raster_png(&bytes) {
            Ok(png) if png.len() <= 2 * 1024 * 1024 && total + png.len() <= 8 * 1024 * 1024 => {
                total += png.len();
                images.resources.insert(
                    cid,
                    format!("data:image/png;base64,{}", STANDARD.encode(png)),
                );
            },
            _ => images.skipped += 1,
        }
    }
    Ok(images)
}

fn raster_png(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    anyhow::ensure!(
        matches!(
            reader.format(),
            Some(
                image::ImageFormat::Png
                    | image::ImageFormat::Jpeg
                    | image::ImageFormat::Gif
                    | image::ImageFormat::WebP
            )
        ),
        "Unsupported embedded image format"
    );
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(32 * 1024 * 1024);
    reader.limits(limits);
    let decoded = reader
        .decode()
        .context("Embedded image could not be decoded")?;
    let mut png = Cursor::new(Vec::new());
    decoded.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Result<Vec<u8>> {
        let image = image::DynamicImage::new_rgb8(width, height);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png)?;
        Ok(bytes.into_inner())
    }

    fn raw_image(bytes: &[u8], copies: usize) -> Vec<u8> {
        let part = format!(
            "--x\r\nContent-Type: image/png\r\nContent-ID: <preview@example.org>\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n",
            STANDARD.encode(bytes)
        );
        format!("Content-Type: multipart/related; boundary=x\r\n\r\n--x\r\nContent-Type: text/html\r\n\r\n<img src=\"cid:preview@example.org\">\r\n{}--x--\r\n",part.repeat(copies)).into_bytes()
    }

    #[test]
    fn links_are_normalized_deduplicated_bounded_and_never_include_local_or_active_schemes()
    -> Result<()> {
        let html = r#"<a href="https://example.org/path?x=1&amp;y=2">Misleading label</a><a href="file:///etc/passwd">file</a><a href="javascript:alert(1)">script</a><a href="//example.org/relative">relative</a><a href="https://user:password@example.org/">credentials</a>"#;
        let found = links(
            "Visit https://example.org/path?x=1&y=2 and https://example.net/docs.",
            Some(html),
        )?;
        assert_eq!(
            found.items,
            [
                "https://example.org/path?x=1&y=2",
                "https://example.net/docs"
            ]
        );
        assert!(!found.truncated);
        for bad in [
            "javascript:alert(1)",
            "file:///tmp/x",
            "data:text/html,hello",
            "mailto:me@example.org",
            "//example.org/x",
            "https://u@example.org",
            "https://example.org/\npath",
        ] {
            assert!(external_url(bad).is_none());
        }
        let many = (0..120)
            .map(|i| format!("https://example.org/{i} "))
            .collect::<String>();
        let found = links(&many, None)?;
        assert_eq!(found.items.len(), 100);
        assert!(found.truncated);
        assert!(links(&"é".repeat(300_000), None)?.truncated);
        Ok(())
    }

    #[test]
    fn cid_images_are_decoded_to_bounded_png_and_only_trusted_resources_reach_html() -> Result<()> {
        let bytes = png(2, 2)?;
        let images = images(&raw_image(&bytes, 1), &moonmail_mime::MailparseParser)?;
        assert_eq!(images.resources.len(), 1);
        let html = crate::page::html_with_images(
            r#"<img src="cid:preview%40example.org"><img src="https://tracker.example/pixel"><img src="data:image/svg+xml,evil"><img src="file:///private/file"><img src="cid:missing"><script>bad()</script>"#,
            &crate::fonts::ReaderFont::default(),
            images.resources,
        );
        assert_eq!(html.matches("src=\"data:image/png;base64,").count(), 1);
        assert!(
            !html.contains("tracker.example")
                && !html.contains("image/svg+xml")
                && !html.contains("file://")
                && !html.contains("<script")
        );
        assert!(!html.contains("cid:"));
        assert!(html.contains("img-src data:"));
        let duplicate = super::images(&raw_image(&bytes, 2), &moonmail_mime::MailparseParser)?;
        assert!(duplicate.resources.is_empty());
        assert_eq!(duplicate.skipped, 1);
        let huge = super::images(
            &raw_image(&png(4097, 1)?, 1),
            &moonmail_mime::MailparseParser,
        )?;
        assert!(huge.resources.is_empty());
        assert_eq!(huge.skipped, 1);
        let vector = super::images(
            &raw_image(
                b"<svg xmlns='http://www.w3.org/2000/svg'><script>bad()</script></svg>",
                1,
            ),
            &moonmail_mime::MailparseParser,
        )?;
        assert!(vector.resources.is_empty());
        assert_eq!(vector.skipped, 1);
        Ok(())
    }
}
