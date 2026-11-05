use {
    anyhow::{Result, bail, ensure},
    serde::{Deserialize, Serialize},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Field {
    Sender,
    Subject,
    Date,
    Recipients,
}

impl Field {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "sender" => Ok(Self::Sender),
            "subject" => Ok(Self::Subject),
            "date" => Ok(Self::Date),
            "recipients" => Ok(Self::Recipients),
            _ => bail!("Unknown message column"),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub field: Field,
    pub width: u16,
    pub visible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Columns(pub Vec<Column>);

impl Default for Columns {
    fn default() -> Self {
        Self(vec![
            Column {
                field: Field::Sender,
                width: 220,
                visible: true,
            },
            Column {
                field: Field::Subject,
                width: 0,
                visible: true,
            },
            Column {
                field: Field::Date,
                width: 156,
                visible: true,
            },
            Column {
                field: Field::Recipients,
                width: 220,
                visible: false,
            },
        ])
    }
}

impl Columns {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.0.len() == 4,
            "Message layout must contain all four columns"
        );
        for (index, column) in self.0.iter().enumerate() {
            ensure!(
                !self.0[..index]
                    .iter()
                    .any(|previous| previous.field == column.field),
                "Duplicate message column"
            );
            if column.field == Field::Subject {
                ensure!(
                    column.visible && column.width == 0,
                    "Subject must remain visible and fill available space"
                );
            } else {
                ensure!(
                    (80..=300).contains(&column.width),
                    "Column width must be between 80 and 300 points"
                );
            }
        }
        ensure!(
            self.0
                .iter()
                .filter(|column| column.visible)
                .map(|column| u32::from(column.width))
                .sum::<u32>()
                <= 700,
            "Visible fixed-width columns may use at most 700 points"
        );
        Ok(())
    }

    /// Produce a candidate so invalid edits never partially modify saved state.
    pub fn edit(&self, operation: &str, field: &str, width: Option<i64>) -> Result<Self> {
        if operation == "reset" {
            return Ok(Self::default());
        }
        let field = Field::parse(field)?;
        let mut next = self.clone();
        let index = next
            .0
            .iter()
            .position(|column| column.field == field)
            .ok_or_else(|| anyhow::anyhow!("Missing column"))?;
        match operation {
            "toggle" => next.0[index].visible = !next.0[index].visible,
            "left" if index > 0 => next.0.swap(index, index - 1),
            "right" if index + 1 < next.0.len() => next.0.swap(index, index + 1),
            "width" => {
                next.0[index].width =
                    u16::try_from(width.ok_or_else(|| anyhow::anyhow!("Choose a column width"))?)?
            },
            _ => bail!("Invalid column edit"),
        }
        next.validate()?;
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn layouts_validate_before_publication_and_legacy_preferences_keep_defaults() -> Result<()> {
        let old: crate::fonts::Preferences = serde_json::from_str(r#"{"mark_read":false}"#)?;
        assert_eq!(old.columns, Columns::default());
        let columns = old
            .columns
            .edit("toggle", "sender", None)?
            .edit("toggle", "recipients", None)?
            .edit("left", "date", None)?
            .edit("width", "recipients", Some(180))?;
        assert!(!columns.0[0].visible);
        assert_eq!(columns.0[1].field, Field::Date);
        for (operation, field, width) in [
            ("toggle", "subject", None),
            ("width", "date", Some(79)),
            ("width", "date", Some(301)),
            ("width", "date", Some(-1)),
            ("width", "subject", Some(100)),
            ("toggle", "unknown", None),
        ] {
            assert!(columns.edit(operation, field, width).is_err());
        }
        let mut invalid = columns.clone();
        invalid.0[0] = invalid.0[1].clone();
        assert!(invalid.validate().is_err());
        let dir = tempfile::tempdir()?;
        let preferences = crate::fonts::Preferences {
            columns: columns.clone(),
            ..old
        };
        preferences.save(dir.path())?;
        assert_eq!(
            crate::fonts::Preferences::load(dir.path())?.columns,
            columns
        );
        assert_eq!(columns.edit("reset", "", None)?, Columns::default());
        Ok(())
    }
}
