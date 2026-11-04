//! The app icon the user picks in Settings. Moonframe sets the Dock icon once,
//! at launch, so a new choice shows the next time Moonmail starts. Sources are
//! `assets/icons/*.svg`; `just icon` renders the PNGs embedded here.

/// One of Moonmail's icons, stored in `preferences.json` by its kebab-case name.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AppIcon {
    /// An @ whose inner "a" holds the moon.
    MoonAt,
    /// A paper plane orbiting the moon.
    Orbit,
    /// A closed envelope sealed with the striped moon: the default. A name
    /// this version does not know, from a newer one, falls back here rather
    /// than failing to load every other preference.
    #[default]
    #[serde(other)]
    WaxSeal,
}

impl AppIcon {
    /// The icon as a 1024 px PNG, for the Dock, the app switcher and About.
    pub fn png(self) -> &'static [u8] {
        match self {
            Self::WaxSeal => include_bytes!("../icons/wax-seal.png"),
            Self::MoonAt => include_bytes!("../icons/moon-at.png"),
            Self::Orbit => include_bytes!("../icons/orbit.png"),
        }
    }

    /// Reads a name the Settings window sends; unknown names are an error.
    pub fn parse(name: &str) -> anyhow::Result<Self> {
        match name {
            "wax-seal" => Ok(Self::WaxSeal),
            "moon-at" => Ok(Self::MoonAt),
            "orbit" => Ok(Self::Orbit),
            other => anyhow::bail!("Unknown app icon: {other}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AppIcon;

    #[test]
    fn names_round_trip_and_unknown_names_fall_back() -> anyhow::Result<()> {
        for icon in [AppIcon::WaxSeal, AppIcon::MoonAt, AppIcon::Orbit] {
            let name = serde_json::to_value(icon)?;
            assert_eq!(AppIcon::parse(name.as_str().unwrap_or_default())?, icon);
            assert!(icon.png().starts_with(b"\x89PNG\r\n\x1a\n"));
        }
        assert!(AppIcon::parse("sunset").is_err());
        let stored: AppIcon = serde_json::from_str("\"from-the-future\"")?;
        assert_eq!(stored, AppIcon::WaxSeal);
        Ok(())
    }
}
