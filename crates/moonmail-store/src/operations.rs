use {
    crate::{Archive, Error, Result},
    moonmail_core::MessageId,
    rusqlite::params,
};

pub struct PendingFlag {
    pub id: i64,
    pub mailbox: String,
    pub epoch: u32,
    pub uid: u32,
    pub kind: String,
    pub value: bool,
}

impl Archive {
    /// Set a flag for up to 100 explicit live messages and all their occurrences.
    /// One invalid/moving message aborts the entire selection and its remote
    /// intents. A grouped representative does not imply its hidden replies.
    pub fn set_flags(&mut self, ids: &[MessageId], kind: &str, value: bool) -> Result<usize> {
        let column = match kind {
            "seen" => "seen",
            "flagged" => "flagged",
            _ => return Err(Error::Invalid("unknown flag")),
        };
        if ids.is_empty() || ids.len() > 100 {
            return Err(Error::Invalid("Choose between one and 100 messages"));
        }
        let ids: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        let tx = self.connection.transaction()?;
        for id in &ids {
            let live: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM occurrences WHERE message_id=?1)",
                [id.0 as i64],
                |row| row.get(0),
            )?;
            if !live {
                return Err(Error::Invalid(
                    "A selected message is no longer in a live folder",
                ));
            }
            let moving:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM move_operations WHERE message_id=?1 AND state IN ('queued','running','unknown'))",[id.0 as i64],|row|row.get(0))?;
            if moving {
                return Err(Error::Invalid(
                    "Finish or cancel selected messages' pending transfers before changing flags",
                ));
            }
            tx.execute("INSERT INTO flag_operations(mailbox_id,epoch,uid,kind,value) SELECT mailbox_id,epoch,uid,?2,?3 FROM occurrences WHERE message_id=?1 AND uid IS NOT NULL ON CONFLICT(mailbox_id,epoch,uid,kind) DO UPDATE SET value=excluded.value",params![id.0 as i64,kind,value])?;
            tx.execute(
                &format!("UPDATE occurrences SET {column}=?2 WHERE message_id=?1"),
                params![id.0 as i64, value],
            )?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    /// Apply to every live member of this account-scoped thread, across folders
    /// and beyond UI page limits. Stage IDs on disk and commit flags/outbox
    /// intents atomically; any pending/uncertain move rejects the whole command.
    pub fn set_thread_seen(&mut self, id: MessageId, seen: bool) -> Result<usize> {
        let transaction = self.connection.transaction()?;
        let component: i64 = transaction.query_row(
            "SELECT n.component FROM thread_members t JOIN thread_nodes n ON n.id=t.node_id WHERE t.message_id=?1",
            [id.0 as i64], |row|row.get(0),
        )?;
        transaction.execute_batch("CREATE TEMP TABLE IF NOT EXISTS flag_targets(id INTEGER PRIMARY KEY); DELETE FROM flag_targets;")?;
        let count = transaction.execute(
            "INSERT INTO flag_targets SELECT t.message_id FROM thread_nodes n JOIN thread_members t ON t.node_id=n.id
             WHERE n.component=?1 AND EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=t.message_id)", [component],
        )?;
        let moving: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM move_operations WHERE message_id IN (SELECT id FROM flag_targets) AND state IN ('queued','running','unknown'))", [], |row|row.get(0),
        )?;
        if moving {
            return Err(Error::Invalid(
                "Finish or cancel pending thread moves before changing flags",
            ));
        }
        transaction.execute(
            "INSERT INTO flag_operations(mailbox_id,epoch,uid,kind,value)
             SELECT mailbox_id,epoch,uid,'seen',?1 FROM occurrences WHERE message_id IN (SELECT id FROM flag_targets) AND uid IS NOT NULL
             ON CONFLICT(mailbox_id,epoch,uid,kind) DO UPDATE SET value=excluded.value", [seen],
        )?;
        transaction.execute(
            "UPDATE occurrences SET seen=?1 WHERE message_id IN (SELECT id FROM flag_targets)",
            [seen],
        )?;
        transaction.execute("DELETE FROM flag_targets", [])?;
        transaction.commit()?;
        Ok(count)
    }

    pub fn set_flag(&mut self, id: MessageId, kind: &str, value: bool) -> Result<()> {
        let column = match kind {
            "seen" => "seen",
            "flagged" => "flagged",
            _ => return Err(Error::Invalid("unknown flag")),
        };
        let transaction = self.connection.transaction()?;
        let moving: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM move_operations WHERE message_id=?1 AND state IN ('queued','running','unknown'))", [id.0 as i64], |row| row.get(0))?;
        if moving {
            return Err(Error::Invalid(
                "Finish or cancel the pending move before changing flags",
            ));
        }
        transaction.execute("INSERT INTO flag_operations(mailbox_id,epoch,uid,kind,value) SELECT mailbox_id,epoch,uid,?2,?3 FROM occurrences WHERE message_id=?1 AND uid IS NOT NULL ON CONFLICT(mailbox_id,epoch,uid,kind) DO UPDATE SET value=excluded.value",params![id.0 as i64,kind,value])?;
        transaction.execute(
            &format!("UPDATE occurrences SET {column}=?2 WHERE message_id=?1"),
            params![id.0 as i64, value],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn pending_flags(&self, account: &str) -> Result<Vec<PendingFlag>> {
        let mut statement = self.connection.prepare("SELECT f.id,b.name,f.epoch,f.uid,f.kind,f.value FROM flag_operations f JOIN mailboxes b ON b.id=f.mailbox_id WHERE b.account=?1 ORDER BY f.id")?;
        Ok(statement
            .query_map([account], |row| {
                Ok(PendingFlag {
                    id: row.get(0)?,
                    mailbox: row.get(1)?,
                    epoch: row.get(2)?,
                    uid: row.get(3)?,
                    kind: row.get(4)?,
                    value: row.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn acknowledge_flag(&self, operation: &PendingFlag) -> Result<()> {
        // A newer click may have replaced the desired value during network I/O.
        self.connection.execute(
            "DELETE FROM flag_operations WHERE id=?1 AND value=?2",
            params![operation.id, operation.value],
        )?;
        Ok(())
    }
}
