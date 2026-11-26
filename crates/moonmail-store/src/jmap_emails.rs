use {
    crate::{Archive, Result},
    moonmail_core::MessageId,
    rusqlite::{OptionalExtension, params},
};

#[derive(Clone, Copy)]
pub struct JmapEmailHandle {
    pub uid: u32,
    pub message: Option<MessageId>,
}

impl Archive {
    /// Resolve a bounded Email/get list in input order, allocating durable
    /// journal UIDs only for previously unseen remote IDs.
    pub fn jmap_email_handles(
        &mut self,
        account: &str,
        ids: &[&str],
    ) -> Result<Vec<JmapEmailHandle>> {
        let ids = serde_json::to_string(ids)?;
        let mut lookup = self.connection.prepare_cached(
            "SELECT e.uid,m.id FROM json_each(?2) q
             LEFT JOIN jmap_emails e ON e.account=?1 AND e.remote=q.value
             LEFT JOIN messages m ON m.account=?1 AND m.origin='jmap:' || q.value
             ORDER BY q.key",
        )?;
        let mut read = || -> rusqlite::Result<Option<Vec<JmapEmailHandle>>> {
            lookup
                .query_map(params![account, ids], |r| {
                    let uid: Option<u32> = r.get(0)?;
                    let message = r.get::<_, Option<i64>>(1)?.map(|id| MessageId(id as u64));
                    Ok(uid.map(|uid| JmapEmailHandle { uid, message }))
                })?
                .collect()
        };
        if let Some(known) = read()? {
            return Ok(known);
        }
        self.connection
            .prepare_cached(
                "INSERT INTO jmap_emails(account,remote) SELECT ?1,q.value FROM json_each(?2) q
             WHERE NOT EXISTS(SELECT 1 FROM jmap_emails e WHERE e.account=?1 AND e.remote=q.value)
             ON CONFLICT(account,remote) DO NOTHING",
            )?
            .execute(params![account, ids])?;
        read()?.ok_or(crate::Error::Invalid("Missing JMAP email handle"))
    }
    pub fn jmap_uid(&self, account: &str, id: &str) -> Result<u32> {
        if let Some(uid) = self
            .connection
            .query_row(
                "SELECT uid FROM jmap_emails WHERE account=?1 AND remote=?2",
                params![account, id],
                |r| r.get(0),
            )
            .optional()?
        {
            return Ok(uid);
        }
        Ok(self.connection.query_row("INSERT INTO jmap_emails(account,remote) VALUES(?1,?2) ON CONFLICT(account,remote) DO UPDATE SET remote=excluded.remote RETURNING uid",params![account,id],|r|r.get(0))?)
    }
    pub fn jmap_remote(&self, account: &str, uid: u32) -> Result<String> {
        Ok(self.connection.query_row(
            "SELECT remote FROM jmap_emails WHERE account=?1 AND uid=?2",
            params![account, uid],
            |r| r.get(0),
        )?)
    }
    pub fn jmap_message(&self, account: &str, id: &str) -> Result<Option<MessageId>> {
        Ok(self
            .connection
            .query_row(
                "SELECT id FROM messages WHERE account=?1 AND origin='jmap:'||?2",
                params![account, id],
                |r| Ok(MessageId(r.get::<_, i64>(0)? as u64)),
            )
            .optional()?)
    }
}
