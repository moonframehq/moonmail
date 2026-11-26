use {
    crate::{Archive, Error, Result},
    moonmail_core::{MailboxRole, MessageId},
    rusqlite::{OptionalExtension, params},
};

pub struct JmapFolder {
    pub remote: String,
    pub path: String,
    pub role: Option<MailboxRole>,
}

impl Archive {
    pub fn bind_jmap(&self, account: &str, remote: &str) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO jmap_accounts(account,remote) VALUES(?1,?2)",
            params![account, remote],
        )?;
        if !self.connection.query_row(
            "SELECT remote=?2 FROM jmap_accounts WHERE account=?1",
            params![account, remote],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Invalid("JMAP primary account changed"));
        }
        Ok(())
    }
    pub fn invalidate_jmap(&self, account: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE jmap_accounts SET mail_state=NULL,folder_state=NULL WHERE account=?1",
            [account],
        )?;
        Ok(())
    }
    pub fn jmap_states(&self, account: &str) -> Result<(Option<String>, Option<String>)> {
        Ok(self.connection.query_row(
            "SELECT folder_state,mail_state FROM jmap_accounts WHERE account=?1",
            [account],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }
    /// Replace a complete mailbox namespace while preserving stable folder IDs across renames.
    pub fn jmap_folders(
        &mut self,
        account: &str,
        folders: &[JmapFolder],
        state: &str,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute("UPDATE mailboxes SET active=0 WHERE account=?1", [account])?;
        tx.execute("UPDATE mailboxes SET name=char(0)||'jmap-'||id WHERE id IN (SELECT mailbox FROM jmap_folders WHERE account=?1)",[account])?;
        for f in folders {
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT mailbox FROM jmap_folders WHERE account=?1 AND remote=?2",
                    params![account, f.remote],
                    |r| r.get(0),
                )
                .optional()?;
            let id = if let Some(id) = existing {
                tx.execute(
                    "UPDATE mailboxes SET name=?2 WHERE id=?1",
                    params![id, f.path],
                )?;
                id
            } else {
                let id=tx.query_row("INSERT INTO mailboxes(account,name) VALUES(?1,?2) ON CONFLICT(account,name) DO UPDATE SET active=1 RETURNING id",params![account,f.path],|r|r.get::<_,i64>(0))?;
                tx.execute(
                    "INSERT INTO jmap_folders VALUES(?1,?2,?3)",
                    params![id, account, f.remote],
                )?;
                id
            };
            tx.execute("UPDATE mailboxes SET active=1,selectable=1,epoch=0,delimiter='/',server_role=?2 WHERE id=?1",params![id,f.role.map(MailboxRole::as_str)])?;
        }
        tx.execute("DELETE FROM occurrences WHERE mailbox_id IN (SELECT id FROM mailboxes WHERE account=?1 AND (active=0 OR occurrences.epoch<>0))",[account])?;
        tx.execute(
            "UPDATE jmap_accounts SET folder_state=?2 WHERE account=?1",
            params![account, state],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn jmap_folder_map(&self, account: &str) -> Result<Vec<(String, i64, String)>> {
        let mut q=self.connection.prepare("SELECT f.remote,b.id,b.name FROM jmap_folders f JOIN mailboxes b ON b.id=f.mailbox WHERE f.account=?1 AND b.active=1")?;
        Ok(
            q.query_map([account], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?,
        )
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
    pub fn observe_jmap(
        &mut self,
        id: MessageId,
        uid: u32,
        folders: &[i64],
        seen: bool,
        flagged: bool,
        keywords: &[String],
    ) -> Result<()> {
        let keywords = crate::keywords::normalize(keywords)?;
        let tx = self.connection.transaction()?;
        let id = id.0 as i64;
        tx.execute("DELETE FROM occurrences WHERE message_id=?1 AND mailbox_id NOT IN (SELECT value FROM json_each(?2))",params![id,serde_json::to_string(folders)?])?;
        for folder in folders {
            let new = tx.execute(
                "INSERT OR IGNORE INTO occurrences VALUES(?1,?2,0,?3,?4,?5)",
                params![folder, id, uid, seen, flagged],
            )?;
            tx.execute(
                "UPDATE occurrences SET
                seen=coalesce((SELECT value FROM flag_operations WHERE mailbox_id=?1 AND epoch=0 AND uid=occurrences.uid AND kind='seen'),?3),
                flagged=coalesce((SELECT value FROM flag_operations WHERE mailbox_id=?1 AND epoch=0 AND uid=occurrences.uid AND kind='flagged'),?4)
                WHERE mailbox_id=?1 AND message_id=?2",
                params![folder, id, seen, flagged],
            )?;
            crate::keywords::replace_observed(&tx, *folder, id, &keywords)?;
            if new > 0 {
                crate::rules::on_entry(&tx, *folder, MessageId(id as u64))?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn begin_jmap_inventory(&self) -> Result<()> {
        self.connection.execute_batch("CREATE TEMP TABLE IF NOT EXISTS jmap_inventory(id TEXT PRIMARY KEY); DELETE FROM jmap_inventory;")?;
        Ok(())
    }
    pub fn stage_jmap_id(&self, id: &str) -> Result<()> {
        self.connection
            .execute("INSERT OR IGNORE INTO jmap_inventory VALUES(?1)", [id])?;
        Ok(())
    }
    pub fn checkpoint_jmap(
        &mut self,
        account: &str,
        state: &str,
        folder_state: Option<&str>,
        destroyed: &[String],
        full: bool,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        if full {
            tx.execute("DELETE FROM occurrences WHERE message_id IN (SELECT id FROM messages WHERE account=?1 AND origin LIKE 'jmap:%' AND substr(origin,6) NOT IN (SELECT id FROM jmap_inventory))",[account])?;
        }
        for id in destroyed {
            tx.execute("DELETE FROM occurrences WHERE message_id=(SELECT id FROM messages WHERE account=?1 AND origin='jmap:'||?2)",params![account,id])?;
        }
        tx.execute(
            "UPDATE jmap_accounts SET mail_state=?2,folder_state=coalesce(?3,folder_state) WHERE account=?1",
            params![account, state,folder_state],
        )?;
        tx.commit()?;
        Ok(())
    }
}

impl Archive {
    pub fn mail_edits_pending(&self, account: &str) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM flag_operations f JOIN mailboxes b ON b.id=f.mailbox_id WHERE b.account=?1 )
            OR EXISTS(SELECT 1 FROM keyword_operations k JOIN mailboxes b ON b.id=k.mailbox_id WHERE b.account=?1 AND k.state='queued')
            OR EXISTS(SELECT 1 FROM move_operations m JOIN mailboxes b ON b.id=m.source_id WHERE b.account=?1 AND m.state='queued')
            OR EXISTS(SELECT 1 FROM folder_operations WHERE account=?1 AND state='queued')",[account],|r|r.get(0))?)
    }
}
