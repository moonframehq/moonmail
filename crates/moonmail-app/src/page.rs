/// Email-controlled navigation, active content and remote resources are removed.
/// Embedded images can only come from trusted decoded/re-encoded CID resources.
#[cfg(test)]
pub fn html(source: &str) -> String {
    html_with_font(source, &crate::fonts::ReaderFont::default())
}

pub fn html_with_font(source: &str, font: &crate::fonts::ReaderFont) -> String {
    html_with_images(source, font, std::collections::HashMap::new())
}

pub fn html_with_images(
    source: &str,
    font: &crate::fonts::ReaderFont,
    images: std::collections::HashMap<String, String>,
) -> String {
    let clean = ammonia::Builder::default()
        .rm_tags(&["a", "form", "input", "video", "audio", "source"])
        .url_schemes(std::collections::HashSet::from(["cid", "data"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .attribute_filter(move |element, attribute, value| {
            if element == "img" && attribute == "src" {
                let prefix = value.get(..4)?;
                if !prefix.eq_ignore_ascii_case("cid:") {
                    return None;
                }
                let cid = percent_encoding::percent_decode_str(&value[4..])
                    .decode_utf8()
                    .ok()?;
                return images
                    .get(cid.as_ref())
                    .cloned()
                    .map(std::borrow::Cow::Owned);
            }
            Some(value.into())
        })
        .clean(source)
        .to_string();
    // CSS string escaping also keeps closing tags out of the style element.
    let family: String = font
        .family
        .chars()
        .map(|ch| format!("\\{:x} ", u32::from(ch)))
        .collect();
    let family = if font.family == ".AppleSystemUIFont" {
        "-apple-system".to_owned()
    } else {
        format!("\"{family}\"")
    };
    let size = font.size.clamp(8, 32);
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"color-scheme\" content=\"light dark\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'\"><style>:root{{color-scheme:light dark;color:#202020;background:#ffffff}}body{{font:{size}px/1.55 {family},sans-serif;max-width:850px;margin:18px}}pre{{white-space:pre-wrap;overflow-wrap:anywhere;font:inherit;margin:0}}table,img{{max-width:100%}}blockquote{{border-left:3px solid #c9c9cd;padding-left:16px;color:#6b6b70}}@media(prefers-color-scheme:dark){{:root{{color:#ededed;background:#202020}}blockquote{{color:#a6a6aa;border-color:#454545}}}}</style></head><body>{clean}</body></html>"
    )
}

#[cfg(test)]
pub fn plain(source: &str) -> String {
    plain_with_font(source, &crate::fonts::ReaderFont::default())
}

pub fn plain_with_font(source: &str, font: &crate::fonts::ReaderFont) -> String {
    let escaped = source
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    html_with_font(&format!("<pre>{escaped}</pre>"), font)
}

#[cfg(test)]
mod tests {
    #[test]
    fn email_cannot_embed_scripts_remote_resources_or_navigation() {
        let rendered = super::html(
            r#"<script>alert(1)</script><img src="https://tracker.example/pixel"><a href="file:///etc/passwd">link</a><iframe src="https://evil.example"></iframe><p onclick="alert(1)" style="background:url(https://tracker.example)">hello</p>"#,
        );
        assert!(!rendered.contains("<script"));
        assert!(!rendered.contains("https://"));
        assert!(!rendered.contains("file:///"));
        assert!(!rendered.contains("onclick"));
        assert!(rendered.contains("default-src 'none'"));
        assert!(rendered.contains("hello"));
        assert!(
            super::plain("<script>literal</script>")
                .contains("&lt;script&gt;literal&lt;/script&gt;")
        );
    }
}
