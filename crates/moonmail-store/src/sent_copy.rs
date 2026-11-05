use {
    crate::{Archive, Error, Result},
    rusqlite::params,
};

impl Archive {
    /// Freeze copy intent before SMTP. SMTP acceptance and this intent then
    /// survive a crash between submission and the separate IMAP operation.
    pub fn prepare_sent_copy(&self, id: i64, append: bool) -> Result<()> {
        let changed = self.connection.execute(
            "UPDATE outbox SET sent_copy_state=?2,sent_copy_detail='',sent_copy_mailbox=''
             WHERE id=?1 AND state='queued' AND sent_copy_state IN ('server','pending')",
            params![id, if append { "pending" } else { "server" }],
        )?;
        if changed != 1 {
            return Err(Error::Invalid(
                "Cannot change this message's Sent copy policy",
            ));
        }
        Ok(())
    }

    pub fn pending_sent_copies(&self, account: &str) -> Result<Vec<i64>> {
        let mut query = self.connection.prepare("SELECT id FROM outbox WHERE account=?1 AND state='sent' AND sent_copy_state='pending' ORDER BY id LIMIT 50")?;
        Ok(query
            .query_map([account], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn claim_sent_copy(&self, id: i64, mailbox: &str) -> Result<bool> {
        if mailbox.is_empty() || mailbox.chars().any(char::is_control) {
            return Err(Error::Invalid("Invalid Sent copy folder"));
        }
        Ok(self.connection.execute(
            "UPDATE outbox SET sent_copy_state='copying',sent_copy_mailbox=?2,sent_copy_detail='Saving accepted message through IMAP'
             WHERE id=?1 AND state='sent' AND sent_copy_state='pending'",
            params![id,mailbox],
        )? == 1)
    }

    pub fn sent_copy_result(&self, id: i64, state: &str, detail: &str) -> Result<()> {
        if !matches!(state, "saved" | "unknown" | "pending") {
            return Err(Error::Invalid("Invalid Sent copy result"));
        }
        self.connection.execute("UPDATE outbox SET sent_copy_state=?2,sent_copy_detail=?3 WHERE id=?1 AND sent_copy_state='copying'", params![id,state,detail])?;
        Ok(())
    }

    pub fn recover_interrupted_sent_copies(&self) -> Result<()> {
        self.connection.execute("UPDATE outbox SET sent_copy_state='unknown',sent_copy_detail='Interrupted while saving Sent copy; Sync and inspect Sent. Never automatically append again.' WHERE sent_copy_state='copying'", [])?;
        Ok(())
    }
}
