use {
    crate::{Archive, Error, Result},
    rusqlite::params,
    serde::Serialize,
};

/// Entries kept; older ones are dropped as new ones arrive.
const KEEP: i64 = 1000;

/// One agent tool call made by a client outside the app.
#[derive(Clone, Debug, Serialize)]
pub struct Activity {
    pub id: i64,
    /// Unix seconds.
    pub at: i64,
    /// Who made it: `cli` or `mcp`.
    pub source: String,
    /// The tool called.
    pub tool: String,
    /// It may change mail or the app, as opposed to only looking.
    pub writes: bool,
    /// It succeeded; otherwise `summary` says why not.
    pub ok: bool,
    pub summary: String,
}

/// A call to log.
pub struct Call<'a> {
    pub source: &'a str,
    pub tool: &'a str,
    pub writes: bool,
    pub ok: bool,
    pub summary: &'a str,
}

impl Archive {
    /// Changes when another connection commits, and never for this one's own
    /// writes, so a reader can poll it cheaply to notice outside changes.
    pub fn data_version(&self) -> Result<i64> {
        Ok(self
            .connection
            .pragma_query_value(None, "data_version", |row| row.get(0))?)
    }

    /// Logged after the change it describes commits: an audit trail for the
    /// user, not part of the change's own atomicity.
    pub fn record_activity(&self, call: &Call<'_>) -> Result<i64> {
        if call.source.is_empty() || call.source.len() > 32 || call.tool.len() > 64 {
            return Err(Error::Invalid("Invalid activity entry"));
        }
        // A long summary (an error chain) is cut, not refused: the log keeps every call.
        let mut end = call.summary.len().min(500);
        while !call.summary.is_char_boundary(end) {
            end -= 1;
        }
        self.connection.execute(
            "INSERT INTO activity(at,source,tool,writes,ok,summary) VALUES (unixepoch(),?1,?2,?3,?4,?5)",
            params![call.source, call.tool, call.writes, call.ok, &call.summary[..end]],
        )?;
        let id = self.connection.last_insert_rowid();
        self.connection
            .execute("DELETE FROM activity WHERE id<=?1", [id - KEEP])?;
        Ok(id)
    }

    /// Entries after `after`, oldest first, at most `limit`.
    pub fn activity_after(&self, after: i64, limit: usize) -> Result<Vec<Activity>> {
        let mut query = self.connection.prepare(
            "SELECT id,at,source,tool,writes,ok,summary FROM activity WHERE id>?1 ORDER BY id LIMIT ?2",
        )?;
        Ok(query
            .query_map(params![after, limit.min(1000) as i64], |row| {
                Ok(Activity {
                    id: row.get(0)?,
                    at: row.get(1)?,
                    source: row.get(2)?,
                    tool: row.get(3)?,
                    writes: row.get(4)?,
                    ok: row.get(5)?,
                    summary: row.get(6)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// The newest entry's ID, or zero.
    pub fn latest_activity(&self) -> Result<i64> {
        Ok(self
            .connection
            .query_row("SELECT COALESCE(MAX(id),0) FROM activity", [], |row| {
                row.get(0)
            })?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call<'a>(source: &'a str, summary: &'a str) -> Call<'a> {
        Call {
            source,
            tool: "set_keyword",
            writes: true,
            ok: true,
            summary,
        }
    }

    #[test]
    fn data_version_moves_only_for_other_connections_and_activity_pages() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let app = Archive::open(dir.path())?;
        let before = app.data_version()?;
        app.record_activity(&call("cli", "own write"))?;
        assert_eq!(
            app.data_version()?,
            before,
            "own commits are not outside changes"
        );
        let agent = Archive::open(dir.path())?;
        agent.record_activity(&call("mcp", "Tagged 2 messages"))?;
        assert_ne!(app.data_version()?, before);
        let entries = app.activity_after(1, 10)?;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].summary, "Tagged 2 messages");
        assert_eq!(app.latest_activity()?, entries[0].id);
        assert!(app.record_activity(&call("", "x")).is_err());
        Ok(())
    }
}
