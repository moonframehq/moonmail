// Only reconstruct inert presentation values. WebKit never receives sender
// stylesheets, URLs, custom properties or arbitrary CSS functions.
pub(super) fn color(input: &str) -> Option<String> {
    let value = input.trim();
    if value.strip_prefix('#').is_some_and(|hex| {
        matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit())
    }) || matches!(value, "transparent" | "white" | "black" | "currentcolor")
    {
        return Some(value.to_owned());
    }
    let (channels, alpha) = if let Some(rest) = value.strip_prefix("rgba(") {
        (rest, true)
    } else {
        (value.strip_prefix("rgb(")?, false)
    };
    let mut parts = channels.strip_suffix(')')?.split(',').map(str::trim);
    let r = parts.next()?.parse::<u8>().ok()?;
    let g = parts.next()?.parse::<u8>().ok()?;
    let b = parts.next()?.parse::<u8>().ok()?;
    let result = if alpha {
        let a = parts.next()?.parse::<f32>().ok()?;
        if !(0.0..=1.0).contains(&a) {
            return None;
        }
        format!("rgba({r},{g},{b},{a})")
    } else {
        format!("rgb({r},{g},{b})")
    };
    parts.next().is_none().then_some(result)
}

fn pixels(value: &str, cap: u16) -> Option<String> {
    if value == "0" {
        return Some("0".into());
    }
    Some(format!(
        "{}px",
        value.strip_suffix("px")?.parse::<u16>().ok()?.min(cap)
    ))
}

fn spacing(value: &str, auto: bool) -> Option<String> {
    let mut values = Vec::new();
    for part in value.split_ascii_whitespace() {
        if values.len() == 4 {
            return None;
        }
        values.push(if auto && part == "auto" {
            "auto".into()
        } else {
            pixels(part, 72)?
        });
    }
    (!values.is_empty()).then(|| values.join(" "))
}

fn border(value: &str) -> Option<String> {
    if matches!(value, "none" | "0") {
        return Some(value.to_owned());
    }
    let mut rest = value.splitn(3, ' ');
    let width = pixels(rest.next()?, 8)?;
    let style = rest.next()?;
    if !matches!(style, "solid" | "dashed" | "dotted" | "double") {
        return None;
    }
    Some(format!("{width} {style} {}", color(rest.next()?)?))
}

fn shadow(value: &str) -> Option<String> {
    let mut rest = value.splitn(4, ' ');
    let x = pixels(rest.next()?, 16)?;
    let y = pixels(rest.next()?, 16)?;
    let blur = pixels(rest.next()?, 24)?;
    Some(format!("{x} {y} {blur} {}", color(rest.next()?)?))
}

fn property(name: &str, value: &str) -> Option<String> {
    match (name, value) {
        ("text-align", "left" | "right" | "center" | "justify")
        | ("font-weight", "normal" | "bold" | "400" | "500" | "600" | "700" | "800")
        | ("font-style", "italic" | "normal")
        | ("border-collapse", "collapse" | "separate")
        | ("display", "none" | "block" | "inline-block") => Some(value.into()),
        ("color" | "background-color" | "background", _) => color(value),
        ("border" | "border-top" | "border-right" | "border-bottom" | "border-left", _) => {
            border(value)
        },
        ("box-shadow", _) => shadow(value),
        ("padding" | "border-radius", _) => spacing(value, false),
        ("margin", _) => spacing(value, true),
        (
            "padding-left" | "padding-right" | "padding-top" | "padding-bottom" | "margin-left"
            | "margin-right" | "margin-top" | "margin-bottom" | "line-height",
            _,
        ) => pixels(value, 96),
        ("width" | "max-width", _) => {
            if let Some(n) = value.strip_suffix('%') {
                Some(format!("{}%", n.parse::<u16>().ok()?.min(100)))
            } else {
                pixels(value, 1200)
            }
        },
        ("font-size", _) => {
            let px = value.strip_suffix("px")?.parse::<u16>().ok()?;
            // Preserve authored sizes at the default 12px reader setting.
            Some(format!("{}rem", f32::from(px.clamp(9, 52)) / 12.0))
        },
        _ => None,
    }
}

pub(super) fn declarations(source: &str) -> Option<String> {
    let mut declarations = Vec::new();
    for declaration in source.split(';') {
        let Some((name, value)) = declaration.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_ascii_lowercase();
        let value = value
            .strip_suffix("!important")
            .unwrap_or(&value)
            .trim_end();
        if let Some(safe) = property(&name, value) {
            let name = if name == "background" {
                "background-color"
            } else {
                &name
            };
            declarations.push(format!("{name}:{safe}"));
        }
    }
    (!declarations.is_empty()).then(|| declarations.join(";"))
}
