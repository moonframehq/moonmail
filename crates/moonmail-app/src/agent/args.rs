use {
    anyhow::{Context, Result, bail},
    moonmail_core::MessageId,
    serde_json::{Map, Value},
};

/// Tool arguments as JSON, from MCP or from parsed command-line flags.
pub struct Args<'a>(pub &'a Map<String, Value>);

impl Args<'_> {
    pub fn text(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(Value::as_str)
    }

    pub fn required(&self, key: &str) -> Result<&str> {
        self.text(key)
            .filter(|value| !value.trim().is_empty())
            .with_context(|| format!("Missing `{key}`"))
    }

    pub fn integer(&self, key: &str) -> Result<Option<i64>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_i64()
                .map(Some)
                .with_context(|| format!("`{key}` must be an integer")),
        }
    }

    pub fn required_integer(&self, key: &str) -> Result<i64> {
        self.integer(key)?
            .with_context(|| format!("Missing `{key}`"))
    }

    pub fn boolean(&self, key: &str) -> Result<Option<bool>> {
        match self.0.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_bool()
                .map(Some)
                .with_context(|| format!("`{key}` must be true or false")),
        }
    }

    /// A limit clamped to `1..=max`.
    pub fn limit(&self, default: usize, max: usize) -> Result<usize> {
        Ok(self
            .integer("limit")?
            .map_or(default, |limit| limit.clamp(1, max as i64) as usize))
    }

    /// A string or an integer, as text: a folder ID or a role name.
    pub fn token(&self, key: &str) -> Option<String> {
        match self.0.get(key)? {
            Value::String(text) => Some(text.clone()),
            Value::Number(number) => Some(number.to_string()),
            _ => None,
        }
    }

    pub fn message(&self, key: &str) -> Result<MessageId> {
        let id = self.required_integer(key)?;
        anyhow::ensure!(id > 0, "`{key}` must be a positive message ID");
        Ok(MessageId(id as u64))
    }

    /// One to 100 message IDs.
    pub fn messages(&self, key: &str) -> Result<Vec<MessageId>> {
        let Some(Value::Array(values)) = self.0.get(key) else {
            bail!("`{key}` must be a list of message IDs");
        };
        anyhow::ensure!(
            (1..=100).contains(&values.len()),
            "Choose between one and 100 messages"
        );
        values
            .iter()
            .map(|value| {
                value
                    .as_i64()
                    .filter(|id| *id > 0)
                    .map(|id| MessageId(id as u64))
                    .with_context(|| format!("`{key}` must hold positive message IDs"))
            })
            .collect()
    }
}
