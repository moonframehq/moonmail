//! Turn a sanitized message into selectable GPUI text and native layout boxes.
//! Sender URLs never become GPUI resources. Explicit CID loads are memory images.
use {
    crate::{fonts::ReaderFont, reader_backend::DocumentSlot},
    base64::{Engine as _, engine::general_purpose::STANDARD},
    kuchikiki::{NodeRef, traits::TendrilSink as _},
    moonframe::{
        Data, HostView,
        gpui_kit::{
            base::{TextView, text::TextViewStyle},
            *,
        },
    },
    std::{collections::HashMap, sync::Arc},
};

#[derive(Default)]
struct BoxStyle {
    background: Option<Rgba>,
    foreground: Option<Rgba>,
    border: Option<(f32, Rgba)>,
    radius: f32,
    shadow: bool,
    padding: [f32; 4],
    max_width: Option<f32>,
    size: Option<f32>,
    bold: bool,
    row: bool,
    center: bool,
    inline_box: bool,
}

fn color(s: &str) -> Option<Rgba> {
    match s {
        "black" => Some(rgb(0)),
        "white" => Some(rgb(0xffffff)),
        "transparent" => Some(rgba(0)),
        _ if s.starts_with("rgb") => {
            let channels = s.split_once('(')?.1.strip_suffix(')')?;
            let mut parts = channels.split(',');
            Some(Rgba {
                r: f32::from(parts.next()?.parse::<u8>().ok()?) / 255.,
                g: f32::from(parts.next()?.parse::<u8>().ok()?) / 255.,
                b: f32::from(parts.next()?.parse::<u8>().ok()?) / 255.,
                a: parts.next().map(str::parse).transpose().ok()?.unwrap_or(1.),
            })
        },
        _ => Rgba::try_from(s).ok(),
    }
}

fn pixels(s: &str) -> Option<f32> {
    s.trim_end_matches("px").parse().ok()
}

impl BoxStyle {
    fn read(node: &kuchikiki::ElementData, font: &ReaderFont) -> Self {
        let attrs = node.attributes.borrow();
        let mut style = Self {
            background: attrs.get("bgcolor").and_then(color),
            max_width: attrs.get("width").and_then(pixels),
            center: attrs.get("align") == Some("center"),
            row: node.name.local.as_ref() == "tr",
            inline_box: node.name.local.as_ref() == "a",
            ..Default::default()
        };
        for (key, value) in attrs
            .get("style")
            .unwrap_or("")
            .split(';')
            .filter_map(|d| d.split_once(':'))
        {
            match key {
                "background-color" => style.background = color(value),
                "color" => style.foreground = color(value),
                "width" | "max-width" => {
                    if let Some(w) = pixels(value) {
                        style.max_width = Some(w)
                    }
                },
                "border" => {
                    let mut words = value.split_whitespace();
                    style.border = words
                        .next()
                        .and_then(pixels)
                        .zip(words.last().and_then(color));
                },
                "border-radius" => {
                    style.radius = value
                        .split_whitespace()
                        .next()
                        .and_then(pixels)
                        .unwrap_or_default()
                },
                "box-shadow" => style.shadow = true,
                "font-size" => {
                    style.size = value
                        .strip_suffix("rem")
                        .and_then(|s| s.parse::<f32>().ok())
                        .map(|n| n * f32::from(font.size))
                },
                "font-weight" => style.bold = matches!(value, "bold" | "600" | "700" | "800"),
                "text-align" => style.center = value == "center",
                "padding" => {
                    let values: Vec<_> = value.split_whitespace().filter_map(pixels).collect();
                    style.padding = match values.as_slice() {
                        [a] => [*a; 4],
                        [a, b] => [*a, *b, *a, *b],
                        [a, b, c] => [*a, *b, *c, *b],
                        [a, b, c, d] => [*a, *b, *c, *d],
                        _ => [0.; 4],
                    };
                },
                "padding-top" => style.padding[0] = pixels(value).unwrap_or_default(),
                "padding-right" => style.padding[1] = pixels(value).unwrap_or_default(),
                "padding-bottom" => style.padding[2] = pixels(value).unwrap_or_default(),
                "padding-left" => style.padding[3] = pixels(value).unwrap_or_default(),
                _ => {},
            }
        }
        style
    }
}

enum Block {
    Text(SharedString),
    Group(BoxStyle, Vec<Block>),
}

pub struct NativeDocument {
    blocks: Vec<Block>,
    font: ReaderFont,
    images: Arc<HashMap<String, Arc<Image>>>,
}

fn flush(text: &mut String, blocks: &mut Vec<Block>) {
    if !text.trim().is_empty() {
        blocks.push(Block::Text(std::mem::take(text).into()));
    }
    text.clear();
}

fn collect(node: &NodeRef, font: &ReaderFont, blocks: &mut Vec<Block>, inline: &mut String) {
    let Some(element) = node.as_element() else {
        inline.push_str(&node.to_string());
        return;
    };
    let tag = element.name.local.as_ref();
    let style = BoxStyle::read(element, font);
    let structural = matches!(
        tag,
        "table"
            | "tbody"
            | "thead"
            | "tfoot"
            | "tr"
            | "td"
            | "th"
            | "div"
            | "section"
            | "article"
            | "center"
    );
    let decorated = style.background.is_some() || style.border.is_some() || style.size.is_some();
    if !structural && !decorated {
        inline.push_str(&node.to_string());
        return;
    }
    flush(inline, blocks);
    let mut children = Vec::new();
    let mut text = String::new();
    for child in node.children() {
        collect(&child, font, &mut children, &mut text);
    }
    flush(&mut text, &mut children);
    if !children.is_empty() {
        blocks.push(Block::Group(style, children));
    }
}

impl NativeDocument {
    #[cfg(test)]
    pub(crate) fn image_count(&self) -> usize {
        self.images.len()
    }

    pub fn plain(source: &str, font: &ReaderFont) -> Self {
        Self {
            blocks: vec![Block::Text(crate::page::plain_fragment(source).into())],
            font: font.clone(),
            images: Default::default(),
        }
    }

    pub fn parse(fragment: &str, font: &ReaderFont) -> Self {
        let tree = kuchikiki::parse_html().one(fragment).document_node;
        let mut images = HashMap::new();
        // Rewrite every image before handing any markup to GPUI's HTML parser.
        if let Ok(nodes) = tree.select("img") {
            for image in nodes.collect::<Vec<_>>() {
                let mut attrs = image.attributes.borrow_mut();
                let bytes = attrs
                    .get("src")
                    .and_then(|s| s.strip_prefix("data:image/png;base64,"))
                    .and_then(|s| STANDARD.decode(s).ok());
                if let Some(bytes) = bytes {
                    let key = format!("moonmail-cid:{}", images.len());
                    images.insert(
                        key.clone(),
                        Arc::new(Image::from_bytes(ImageFormat::Png, bytes)),
                    );
                    attrs.insert("src", key);
                } else {
                    if let Some(alt) = attrs.get("alt") {
                        image.as_node().insert_before(NodeRef::new_text(alt));
                    }
                    image.as_node().detach();
                }
            }
        }
        if let Ok(nodes) = tree.select("[style]") {
            for entry in nodes.collect::<Vec<_>>() {
                if entry
                    .attributes
                    .borrow()
                    .get("style")
                    .is_some_and(|s| s.split(';').any(|d| d == "display:none"))
                {
                    entry.as_node().detach();
                }
            }
        }
        let mut blocks = Vec::new();
        let mut inline = String::new();
        if let Ok(body) = tree.select_first("body") {
            // Bound layout depth and element count before recursive serialization
            // or creation of GPUI entities. Keep the text readable for oversized mail.
            let complex = body
                .as_node()
                .descendants()
                .filter(|n| n.as_element().is_some())
                .enumerate()
                .any(|(i, n)| i >= 1500 || n.ancestors().take(49).count() > 48);
            if complex {
                let text = body
                    .as_node()
                    .descendants()
                    .filter_map(|n| n.as_text().map(|t| t.borrow().to_string()))
                    .collect::<Vec<_>>()
                    .join(" ");
                inline = crate::page::plain_fragment(&text);
            } else {
                for node in body.as_node().children() {
                    collect(&node, font, &mut blocks, &mut inline);
                }
            }
        }
        flush(&mut inline, &mut blocks);
        Self {
            blocks,
            font: font.clone(),
            images: Arc::new(images),
        }
    }

    fn text_view(&self, html: &SharedString, index: usize, foreground: Hsla) -> TextView {
        let images = self.images.clone();
        TextView::html(("body", index), html.clone())
            .style(TextViewStyle::default().with_foreground(foreground))
            .selectable(true)
            .on_link_click(|_, _, _, _| {})
            .image_source(move |uri| {
                images
                    .get(uri.as_ref())
                    .cloned()
                    .map(ImageSource::Image)
                    .unwrap_or_else(|| ImageSource::Custom(Arc::new(|_, _| None)))
            })
    }

    fn block(&self, block: &Block, index: &mut usize, foreground: Hsla) -> AnyElement {
        *index += 1;
        match block {
            Block::Text(html) => self
                .text_view(html, *index, foreground)
                .scrollable(false)
                .into_any_element(),
            Block::Group(style, children) => {
                let fg = style.foreground.map(Into::into).unwrap_or(foreground);
                let [t, r, b, l] = style.padding;
                let mut el = div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .max_w_full()
                    .pt(px(t))
                    .pr(px(r))
                    .pb(px(b))
                    .pl(px(l));
                if style.row {
                    el = el.flex_row();
                }
                if style.center {
                    el = el.items_center();
                }
                if let Some(w) = style.max_width {
                    el = el.w(px(w));
                } else if style.inline_box {
                    el = el.self_start();
                } else {
                    el = el.w_full();
                }
                if let Some(c) = style.background {
                    el = el.bg(c);
                }
                if let Some((w, c)) = style.border {
                    el = el.border(px(w)).border_color(c);
                }
                if style.radius > 0. {
                    el = el.rounded(px(style.radius));
                }
                if style.shadow {
                    el = el.shadow_sm();
                }
                if let Some(s) = style.size {
                    el = el.text_size(px(s));
                }
                if style.bold {
                    el = el.font_weight(FontWeight::BOLD);
                }
                el.children(children.iter().map(|c| self.block(c, index, fg)))
                    .into_any_element()
            },
        }
    }
    fn element(&self, id: u64, theme: &base::Theme) -> AnyElement {
        let mut index = 0;
        let canvas = self.blocks.iter().find_map(|block| match block {
            Block::Group(style, _) => style.background,
            _ => None,
        });
        let (fg, bg) = if let Some(bg) = canvas {
            (rgb(0x202124).into(), bg.into())
        } else {
            (
                theme.tokens.colors.foreground,
                theme.tokens.colors.background,
            )
        };
        let root = div()
            .id(("reader-scroll", id))
            .size_full()
            .bg(bg)
            .text_size(px(f32::from(self.font.size)))
            .font_family(self.font.family.clone());
        if let [Block::Text(text)] = self.blocks.as_slice() {
            // Let GPUI virtualize the visible lines of long text documents.
            return root
                .p_3()
                .child(self.text_view(text, 0, fg).scrollable(true).size_full())
                .into_any_element();
        }
        let mut column = div().w_full().flex().flex_col();
        if canvas.is_none() {
            column = column.p_3();
        }
        root.overflow_y_scroll()
            .child(column.children(self.blocks.iter().map(|b| self.block(b, &mut index, fg))))
            .into_any_element()
    }
}

pub struct ReaderView {
    slot: DocumentSlot,
    document: Option<Arc<NativeDocument>>,
    scroll_id: u64,
}
impl ReaderView {
    pub fn new(slot: DocumentSlot, props: &Data) -> Self {
        let mut view = Self {
            slot,
            document: None,
            scroll_id: 0,
        };
        view.read(props);
        view
    }
    fn read(&mut self, props: &Data) {
        let revision = props
            .get("revision")
            .and_then(Data::as_i64)
            .unwrap_or_default() as u64;
        let rich = props.get("rich").and_then(Data::as_bool) == Some(true);
        self.scroll_id = revision.wrapping_mul(2) | u64::from(rich);
        self.document = self.slot.read(revision, rich);
    }
}
impl HostView for ReaderView {
    fn set_props(&mut self, props: &Data, _: &mut Context<Self>) {
        self.read(props);
    }
}
impl Render for ReaderView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.document
            .as_ref()
            .map(|d| d.element(self.scroll_id, &base::Theme::global(cx)))
            .unwrap_or_else(|| div().into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::{Block, HashMap, NativeDocument, ReaderFont};

    #[test]
    fn only_decoded_cid_resources_enter_the_native_image_map() {
        let source = r#"<img src="https://invalid.example/a" alt="Blocked logo"><img src="file:///never"><img src="data:image/png;base64,YQ=="><img src="cid:logo"><span style="display:none">Preview</span>"#;
        let fragment = crate::page::sanitized(
            source,
            HashMap::from([("logo".into(), "data:image/png;base64,YQ==".into())]),
        );
        let doc = NativeDocument::parse(&fragment, &ReaderFont::default());
        assert_eq!(doc.images.len(), 1);
        let [Block::Text(html)] = doc.blocks.as_slice() else {
            panic!("expected one text block")
        };
        assert!(html.contains("Blocked logo") && html.contains("moonmail-cid:0"));
        assert!(
            !html.contains("Preview")
                && !html.contains("data:image")
                && !html.contains("file:")
                && !html.contains("https:")
        );
    }
}
