use {
    crate::{Archive, Error, Result},
    rusqlite::{OptionalExtension, params},
    serde::Serialize,
};

/// Requests kept; older finished ones are dropped as new ones arrive.
const KEEP: i64 = 500;

/// Something an outside client asked the open app to do.
///
/// States: `pending` until the app acts (a `send` until the user answers);
/// then `done`, `denied` (by the user), `failed`, or `expired` (superseded, or
/// left from an earlier session). `detail` holds an answer or the error.
#[derive(Clone, Debug, Serialize)]
pub struct AgentRequest {
    pub id: i64,
    /// Unix seconds.
    pub at: i64,
    pub source: String,
    /// `open_draft`, `show_message`, `show_search`, `sync`, `state` (what
    /// the window shows), `action` (a UI action, as a click sends it), or
    /// `send` (a draft to send once the user approves it).
    pub kind: String,
    /// A draft ID, message ID, query or action JSON, by kind.
    pub target: String,
    pub summary: String,
    pub state: String,
    pub detail: String,
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRequest> {
    Ok(AgentRequest {
        id: row.get(0)?,
        at: row.get(1)?,
        source: row.get(2)?,
        kind: row.get(3)?,
        target: row.get(4)?,
        summary: row.get(5)?,
        state: row.get(6)?,
        detail: row.get(7)?,
    })
}

const SELECT: &str = "SELECT id,at,source,kind,target,summary,state,detail FROM agent_requests";

impl Archive {
    pub fn add_request(
        &self,
        source: &str,
        kind: &str,
        target: &str,
        summary: &str,
    ) -> Result<i64> {
        if source.is_empty()
            || kind.is_empty()
            || kind.len() > 32
            || target.len() > 64 * 1024
            || summary.len() > 500
        {
            return Err(Error::Invalid("Invalid agent request"));
        }
        self.connection.execute(
            "INSERT INTO agent_requests(at,source,kind,target,summary) VALUES (unixepoch(),?1,?2,?3,?4)",
            params![source, kind, target, summary],
        )?;
        let id = self.connection.last_insert_rowid();
        self.connection.execute(
            "DELETE FROM agent_requests WHERE id<=?1 AND state<>'pending'",
            [id - KEEP],
        )?;
        Ok(id)
    }

    pub fn agent_request(&self, id: i64) -> Result<Option<AgentRequest>> {
        Ok(self
            .connection
            .query_row(&format!("{SELECT} WHERE id=?1"), [id], decode)
            .optional()?)
    }

    /// Pending requests, oldest first.
    pub fn pending_requests(&self) -> Result<Vec<AgentRequest>> {
        let mut query = self.connection.prepare(&format!(
            "{SELECT} WHERE state='pending' ORDER BY id LIMIT 100"
        ))?;
        Ok(query
            .query_map([], decode)?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Settle a pending request; false when it was no longer pending.
    pub fn settle_request(&self, id: i64, state: &str, detail: &str) -> Result<bool> {
        if !matches!(state, "done" | "denied" | "failed" | "expired") || detail.len() > 256 * 1024 {
            return Err(Error::Invalid("Invalid agent request outcome"));
        }
        Ok(self.connection.execute(
            "UPDATE agent_requests SET state=?2,detail=?3 WHERE id=?1 AND state='pending'",
            params![id, state, detail],
        )? == 1)
    }

    /// Expire pending requests of `kind` on `target`, such as a send request
    /// once the agent changes its draft. Returns how many.
    pub fn expire_requests(&self, kind: &str, target: &str, detail: &str) -> Result<usize> {
        Ok(self.connection.execute(
            "UPDATE agent_requests SET state='expired',detail=?3 WHERE state='pending' AND kind=?1 AND target=?2",
            params![kind, target, detail],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_settle_once_and_expire_by_target() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let archive = Archive::open(dir.path())?;
        let open = archive.add_request("mcp", "open_draft", "draft-1", "Open a draft")?;
        let show = archive.add_request("cli", "show_message", "4", "Show a message")?;
        assert_eq!(archive.pending_requests()?.len(), 2);
        assert!(archive.settle_request(show, "done", "")?);
        assert!(
            !archive.settle_request(show, "failed", "late")?,
            "settled once"
        );
        assert_eq!(
            archive.expire_requests("open_draft", "draft-1", "Draft discarded")?,
            1
        );
        let request = archive.agent_request(open)?.ok_or(Error::NotFound)?;
        assert_eq!(
            (request.state.as_str(), request.detail.as_str()),
            ("expired", "Draft discarded")
        );
        assert!(archive.pending_requests()?.is_empty());
        assert!(archive.settle_request(open, "approved", "").is_err());
        Ok(())
    }
}
