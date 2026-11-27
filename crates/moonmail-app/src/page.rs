/// Email-controlled navigation, active content and remote resources are removed.
/// Images can only come from locally decoded and re-encoded resources.
#[cfg(test)]
pub fn html(source: &str) -> String {
    html_with_font(source, &crate::fonts::ReaderFont::default())
}

pub fn html_with_font(source: &str, font: &crate::fonts::ReaderFont) -> String {
    html_with_images(source, font, std::collections::HashMap::new())
}

#[path = "reader_style.rs"]
mod style;

pub fn html_with_images(
    source: &str,
    font: &crate::fonts::ReaderFont,
    images: std::collections::HashMap<String, String>,
) -> String {
    let clean = sanitized(source, images);
    document(&clean, font)
}

pub fn sanitized(source: &str, images: std::collections::HashMap<String, String>) -> String {
    sanitize_images(source, images.into(), Default::default())
}

pub(super) fn sanitize_images(
    source: &str,
    embedded: std::sync::Arc<std::collections::HashMap<String, String>>,
    remote: std::sync::Arc<std::collections::HashMap<String, String>>,
) -> String {
    // Inline only local style blocks. Large messages still get their inline
    // formatting without paying selector-matching cost on the selection path.
    let inlined = (source.len() <= 256 * 1024
        && source
            .as_bytes()
            .windows(6)
            .any(|prefix| prefix.eq_ignore_ascii_case(b"<style")))
    .then(|| {
        css_inline::CSSInliner::options()
            .load_remote_stylesheets(false)
            .build()
            .inline(source)
            .ok()
    })
    .flatten();
    // Repeated references must not expand a small message into an unbounded document.
    let remaining_bytes = std::sync::atomic::AtomicUsize::new(24 * 1024 * 1024);
    let inline_png = move |data: &String| {
        remaining_bytes
            .try_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |left| left.checked_sub(data.len()),
            )
            .ok()?;
        Some(std::borrow::Cow::Owned(data.clone()))
    };
    ammonia::Builder::default()
        .rm_tags(&["form", "input", "video", "audio", "source"])
        .rm_tag_attributes("a", &["href", "hreflang"])
        .link_rel(None)
        .add_tag_attributes("table", &["bgcolor", "width", "cellspacing", "cellpadding"])
        .add_tag_attributes("td", &["bgcolor", "width", "valign"])
        .add_tag_attributes("th", &["bgcolor", "width", "valign"])
        .add_tag_attributes("tr", &["bgcolor"])
        .add_generic_attributes(&["style"])
        .url_schemes(std::collections::HashSet::from(["cid", "http", "https"]))
        .url_relative(ammonia::UrlRelative::Deny)
        .attribute_filter(move |element, attribute, value| {
            if attribute == "style" {
                return style::declarations(value).map(std::borrow::Cow::Owned);
            }
            if attribute == "bgcolor" {
                return style::color(value).map(std::borrow::Cow::Owned);
            }
            if matches!(attribute, "width" | "cellpadding" | "cellspacing") {
                let (digits, suffix) = value
                    .trim()
                    .strip_suffix('%')
                    .map_or((value.trim(), ""), |v| (v, "%"));
                let limit = if suffix == "%" {
                    100
                } else if attribute == "width" {
                    1200
                } else {
                    64
                };
                return Some(format!("{}{suffix}", digits.parse::<u16>().ok()?.min(limit)).into());
            }
            if element == "img" && attribute == "src" {
                let prefix = value.get(..4)?;
                if !prefix.eq_ignore_ascii_case("cid:") {
                    if remote.is_empty() {
                        return None;
                    }
                    return remote
                        .get(&crate::remote_images::image_url(value)?)
                        .and_then(&inline_png);
                }
                let cid = percent_encoding::percent_decode_str(&value[4..])
                    .decode_utf8()
                    .ok()?;
                return embedded.get(cid.as_ref()).and_then(&inline_png);
            }
            if matches!(
                attribute,
                "href" | "src" | "cite" | "ping" | "action" | "poster" | "data" | "formaction"
            ) {
                return None;
            }
            Some(value.into())
        })
        .clean(inlined.as_deref().unwrap_or(source))
        .to_string()
}

pub fn document(clean: &str, font: &crate::fonts::ReaderFont) -> String {
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
    let stylesheet = include_str!("reader.css");
    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><meta name="color-scheme" content="light dark"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="referrer" content="no-referrer"><meta http-equiv="Content-Security-Policy" content="default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'"><style>:root{{font-size:{size}px;font-family:{family},sans-serif}}{stylesheet}</style></head><body>{clean}</body></html>"#
    )
}

#[cfg(test)]
pub fn plain(source: &str) -> String {
    plain_with_font(source, &crate::fonts::ReaderFont::default())
}

#[cfg(test)]
pub fn plain_with_font(source: &str, font: &crate::fonts::ReaderFont) -> String {
    document(&plain_fragment(source), font)
}

pub fn plain_fragment(source: &str) -> String {
    let escaped = source
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<pre>{escaped}</pre>")
}

#[cfg(test)]
mod tests {
    #[test]
    fn local_styles_and_legacy_table_colors_survive_without_external_resources() {
        let result = super::html(
            r##"<link rel="stylesheet" href="file:///never.css"><style>
            @import url(https://assets.example/never.css);
            .card {border:1px solid #DCDFE3; padding:32px 24px; box-shadow:0px 0px 4px rgba(69, 75, 82, 0.25); background-image:url(https://assets.example/pixel)}
            .cta {background:#0057b8; color:#fff; display:inline-block}
            </style><table bgcolor="#f8fafb" width="100%"><tr><td class="card" bgcolor="#ffffff"><a class="cta" href="https://assets.example/open">Review</a></td></tr></table>"##,
        );
        for kept in [
            "border:1px solid #dcdfe3",
            "padding:32px 24px",
            "box-shadow:0px 0px 4px rgba(69,75,82,0.25)",
            "bgcolor=\"#f8fafb\"",
            "width=\"100%\"",
            "background-color:#0057b8",
            ">Review</a>",
        ] {
            assert!(result.contains(kept), "missing {kept}");
        }
        for removed in ["href=", "assets.example", "never.css", "@import"] {
            assert!(!result.contains(removed), "retained {removed}");
        }
    }

    #[test]
    fn receipt_preserves_type_and_table_structure_without_resource_attributes() {
        let page = super::html(
            r##"<table><tr><td style="background-color:#f2f4f6;padding:24px">
            <span style="font-size:24px;font-weight:700;background-image:url(https://images.example/banner)">Payment received</span>
            <picture><source srcset="//images.example/banner"><img alt="Receipt" width="800" height="600" srcset="//images.example/logo 2x" src="https://images.example/logo"></picture>
            </td></tr></table>"##,
        );
        assert!(page.contains("<table>"));
        assert!(page.contains("background-color:#f2f4f6;padding:24px"));
        assert!(page.contains("font-size:2rem;font-weight:700"));
        assert!(page.contains("alt=\"Receipt\""));
        assert!(!page.contains("images.example"));
        assert!(!page.contains("srcset"));
        assert_eq!(
            super::style::declarations(
                "color:var(--ink); font-size:calc(1em + 2px); padding:inherit"
            ),
            None
        );
    }

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
