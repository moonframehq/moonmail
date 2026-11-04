use {
    anyhow::Result,
    std::{
        io::Write,
        path::{Path, PathBuf},
    },
};

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct ReaderFont {
    pub family: String,
    pub size: u16,
}

impl Default for ReaderFont {
    fn default() -> Self {
        Self {
            family: "Helvetica".into(),
            size: 12,
        }
    }
}

impl ReaderFont {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.family.trim().is_empty()
                && self.family.len() <= 100
                && !self.family.chars().any(char::is_control),
            "Enter a font family (up to 100 characters)"
        );
        anyhow::ensure!(
            (8..=32).contains(&self.size),
            "Reader font size must be between 8 and 32"
        );
        Ok(())
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub interface_family: String,
    pub list_family: String,
    pub font_size: u16,
    pub reader: ReaderFont,
    pub threaded: bool,
    pub mark_read: bool,
    pub app_icon: crate::app_icon::AppIcon,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            interface_family: ".AppleSystemUIFont".into(),
            list_family: "JetBrains Mono".into(),
            font_size: 12,
            reader: ReaderFont::default(),
            threaded: false,
            mark_read: true,
            app_icon: crate::app_icon::AppIcon::default(),
        }
    }
}

impl Preferences {
    pub fn load(root: &Path) -> Result<Self> {
        match std::fs::read(root.join("preferences.json")) {
            Ok(bytes) => {
                let settings: Self = serde_json::from_slice(&bytes)?;
                settings.validate()?;
                Ok(settings)
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }
    pub fn validate(&self) -> Result<()> {
        for family in [&self.interface_family, &self.list_family] {
            anyhow::ensure!(
                !family.trim().is_empty()
                    && family.len() <= 100
                    && !family.chars().any(char::is_control),
                "Enter a font family (up to 100 characters)"
            );
        }
        anyhow::ensure!(
            (10..=16).contains(&self.font_size),
            "Interface font size must be between 10 and 16"
        );
        self.reader.validate()
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        self.validate()?;
        atomic_write(
            &root.join("preferences.json"),
            &serde_json::to_vec_pretty(self)?,
        )
    }
    pub fn apply_fonts(&self, target: &Path) -> Result<()> {
        self.validate()?;
        let original = match std::fs::read_to_string(target) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                "return style {}\n".into()
            },
            Err(error) => return Err(error.into()),
        };
        let original = if original.starts_with(BEGIN) {
            original
                .split_once(END)
                .map(|(_, rest)| rest)
                .ok_or_else(|| anyhow::anyhow!("Incomplete Moonmail font settings block"))?
        } else {
            &original
        };
        // A lexical wrapper changes only fonts. Preserve the user's entire
        // existing stylesheet verbatim below the managed block.
        let interface = serde_json::to_string(&self.interface_family)?;
        let list = serde_json::to_string(&self.list_family)?;
        let size = self.font_size;
        let text = format!(
            "{BEGIN}local moonmail_style = style\nlocal function style(sheet)\n    sheet.font_family = {interface}\n    sheet.font_size = {size}\n    sheet.mono = sheet.mono or {{}}\n    sheet.mono.family = {list}\n    sheet.mono.size = {size}\n    return moonmail_style(sheet)\nend\n{END}{original}"
        );
        atomic_write(target, text.as_bytes())
    }
}

const BEGIN: &str = "-- BEGIN Moonmail managed fonts\n";
const END: &str = "-- END Moonmail managed fonts\n";

fn atomic_write(target: &Path, bytes: &[u8]) -> Result<()> {
    let parent = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Missing settings directory"))?;
    std::fs::create_dir_all(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(target)?;
    Ok(())
}

pub fn path(demo: bool) -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    config
        .join(if demo { "moonmail-demo" } else { "moonmail" })
        .join("style.luau")
}

/// Query Core Text on the provider worker, once per settings session.
pub fn installed() -> Vec<String> {
    #[cfg(target_os = "macos")]
    let mut families: Vec<String> = core_text::font_collection::get_family_names()
        .iter()
        .map(|name| name.to_string())
        .filter(|name| !name.starts_with('.') && name.len() <= 100)
        .collect();
    #[cfg(not(target_os = "macos"))]
    let mut families: Vec<String> = vec!["Helvetica".into(), "Menlo".into()];
    families.sort_by_key(|name| name.to_lowercase());
    families.dedup();
    families.insert(0, ".AppleSystemUIFont".into());
    families
}

pub fn preview(font: &ReaderFont) -> Result<String> {
    font.validate()?;
    Ok(crate::page::html_with_font(
        "<p>The quick brown fox jumps over the lazy dog.</p><p>ABCDEFGHIJKLMNOPQRSTUVWXYZ<br>abcdefghijklmnopqrstuvwxyz<br>0123456789 &amp; @ # % + − =</p><hr><p>Clara Chen<br>A quieter place to work</p><p>Hi Alex,<br><br>Here are my notes for Thursday. A little more space, a little less noise.<br><br>See you soon,<br>Clara</p>",
        font,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn font_changes_preserve_custom_styles_and_roundtrip() -> Result<()> {
        let root = tempfile::tempdir()?;
        let target = root.path().join("style.luau");
        let custom = "-- My colors\nreturn style { colors = { accent = '#123456' } }\n";
        std::fs::write(&target, custom)?;
        let mut settings = Preferences::default();
        settings.apply_fonts(&target)?;
        settings.list_family = "Menlo".into();
        settings.apply_fonts(&target)?;
        let text = std::fs::read_to_string(&target)?;
        assert!(text.ends_with(custom));
        assert_eq!(text.matches(BEGIN).count(), 1);
        assert!(text.contains("\"Menlo\""));
        let workspace = moonframe::moonframe_core::Workspace::open(
            moonframe::moonframe_core::Roots::new([root.path().to_owned()]),
            false,
        );
        assert!(
            workspace.style_error().is_none(),
            "{:?}",
            workspace.style_error()
        );
        settings.save(root.path())?;
        assert_eq!(Preferences::load(root.path())?.list_family, "Menlo");
        settings.font_size = 100;
        assert!(settings.apply_fonts(&target).is_err());
        assert_eq!(std::fs::read_to_string(&target)?, text);
        Ok(())
    }
}
