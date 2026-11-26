//! Main window layout presets selectable from the macOS View menu and Settings.

/// One of Moonmail's main window layouts, stored in `preferences.json`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// Compact unified toolbar, slim workspace rail, and MailMate correspondence split.
    #[serde(rename = "mailmate")]
    MailMate,
    /// Settings-matched modern chrome with tinted sidebar icon badges and Correspondence split.
    Moonmail,
    /// Apple Mail–style capsule toolbar, activity bar + sidebar, and 3-column reading view.
    #[default]
    #[serde(other)]
    Mail,
}

impl Layout {
    pub fn parse(name: &str) -> anyhow::Result<Self> {
        match name {
            "mailmate" => Ok(Self::MailMate),
            "mail" => Ok(Self::Mail),
            "moonmail" => Ok(Self::Moonmail),
            other => anyhow::bail!("Unknown layout: {other}"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MailMate => "MailMate",
            Self::Mail => "Mail.app",
            Self::Moonmail => "Moonmail",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Layout;

    #[test]
    fn names_round_trip_and_unknown_names_fall_back() -> anyhow::Result<()> {
        for layout in [Layout::MailMate, Layout::Mail, Layout::Moonmail] {
            let name = serde_json::to_value(layout)?;
            assert_eq!(Layout::parse(name.as_str().unwrap_or_default())?, layout);
            assert!(!layout.label().is_empty());
        }
        assert!(Layout::parse("classic").is_err());
        let stored: Layout = serde_json::from_str("\"from-the-future\"")?;
        assert_eq!(stored, Layout::Mail);
        Ok(())
    }
}
