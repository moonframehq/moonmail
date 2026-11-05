use {
    crate::{Archive, Error, Result},
    rusqlite::params,
};

/// Metadata only; bodies are downloaded individually after a complete inventory.
pub struct SyncMessage {
    pub uid: u32,
    pub seen: bool,
    pub starred: bool,
    pub size: u32,
    pub received_at: i64,
    pub keywords: Vec<String>,
}

impl Archive {
    /// Connection-local disk-backed staging. Starting a scan discards any partial
    /// previous scan, but never changes authoritative occurrences or checkpoints.
    pub fn begin_sync_inventory(&self) -> Result<()> {
        self.connection.execute_batch("CREATE TEMP TABLE IF NOT EXISTS sync_inventory (
            uid INTEGER PRIMARY KEY, seen INTEGER NOT NULL, flagged INTEGER NOT NULL,
             size INTEGER NOT NULL, received_at INTEGER NOT NULL);
             CREATE TEMP TABLE IF NOT EXISTS sync_keywords (uid INTEGER NOT NULL, keyword TEXT NOT NULL, PRIMARY KEY(uid,keyword));
            CREATE TEMP TABLE IF NOT EXISTS sync_scope (epoch INTEGER, upper_uid INTEGER, mailbox INTEGER, complete INTEGER);
             DELETE FROM sync_inventory; DELETE FROM sync_scope; DELETE FROM sync_keywords;")?;
        Ok(())
    }

    /// Every batch must belong to the same SELECT epoch and fixed UID upper bound.
    /// An empty first batch establishes scope even for an empty mailbox.
    pub fn stage_sync_inventory(
        &mut self,
        epoch: u32,
        upper_uid: u32,
        messages: &[SyncMessage],
    ) -> Result<()> {
        if epoch == 0
            || messages.len() > 1000
            || messages.iter().any(|m| m.uid == 0 || m.uid > upper_uid)
        {
            return Err(Error::Invalid("Invalid sync inventory batch"));
        }
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO sync_scope SELECT ?1,?2,NULL,0 WHERE NOT EXISTS(SELECT 1 FROM sync_scope)",
            params![epoch, upper_uid],
        )?;
        let matches: bool = transaction.query_row(
            "SELECT epoch=?1 AND upper_uid=?2 AND complete=0 FROM sync_scope",
            params![epoch, upper_uid],
            |row| row.get(0),
        )?;
        if !matches {
            return Err(Error::Invalid("Sync inventory scope changed"));
        }
        {
            let mut insert =
                transaction.prepare_cached("INSERT INTO sync_inventory VALUES (?1,?2,?3,?4,?5)")?;
            for message in messages {
                for keyword in crate::keywords::normalize(&message.keywords)? {
                    transaction.execute(
                        "INSERT INTO sync_keywords VALUES (?1,?2)",
                        params![message.uid, keyword],
                    )?;
                }
                insert.execute(params![
                    message.uid,
                    message.seen,
                    message.starred,
                    message.size,
                    message.received_at
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    /// Call only after the adapter has successfully completed the entire scan.
    /// Apply epoch, expunges and remote flags atomically; pending local flag intent
    /// wins. Bytes remain archived. Newer UIDs outside the scan are not retired.
    pub fn finish_sync_inventory(&mut self, mailbox: i64) -> Result<u32> {
        let transaction = self.connection.transaction()?;
        let (epoch, upper, complete): (u32, u32, bool) = transaction.query_row(
            "SELECT epoch,upper_uid,complete FROM sync_scope",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if complete {
            return Err(Error::Invalid("Sync inventory already applied"));
        }
        if transaction.execute(
            "UPDATE mailboxes SET epoch=?2 WHERE id=?1",
            params![mailbox, epoch],
        )? != 1
        {
            return Err(Error::NotFound);
        }
        transaction.execute("DELETE FROM occurrences WHERE mailbox_id=?1 AND (epoch<>?2 OR (uid<=?3 AND uid NOT IN (SELECT uid FROM sync_inventory)))",params![mailbox,epoch,upper])?;
        transaction.execute("UPDATE occurrences AS o SET
            seen=CASE WHEN EXISTS(SELECT 1 FROM flag_operations f WHERE f.mailbox_id=o.mailbox_id AND f.epoch=o.epoch AND f.uid=o.uid AND f.kind='seen') THEN seen ELSE (SELECT seen FROM sync_inventory s WHERE s.uid=o.uid) END,
            flagged=CASE WHEN EXISTS(SELECT 1 FROM flag_operations f WHERE f.mailbox_id=o.mailbox_id AND f.epoch=o.epoch AND f.uid=o.uid AND f.kind='flagged') THEN flagged ELSE (SELECT flagged FROM sync_inventory s WHERE s.uid=o.uid) END
            WHERE mailbox_id=?1 AND epoch=?2 AND uid IN (SELECT uid FROM sync_inventory)",params![mailbox,epoch])?;
        transaction.execute("UPDATE sync_scope SET mailbox=?1,complete=1", [mailbox])?;
        transaction.execute("DELETE FROM occurrence_keywords AS k WHERE k.mailbox_id=?1
            AND EXISTS(SELECT 1 FROM occurrences o JOIN sync_inventory s ON s.uid=o.uid WHERE o.mailbox_id=k.mailbox_id AND o.message_id=k.message_id AND o.epoch=?2)
            AND NOT EXISTS(SELECT 1 FROM keyword_operations f JOIN occurrences o ON o.mailbox_id=f.mailbox_id AND o.epoch=f.epoch AND o.uid=f.uid
                WHERE o.mailbox_id=k.mailbox_id AND o.message_id=k.message_id AND f.keyword=k.keyword)",params![mailbox,epoch])?;
        transaction.execute("INSERT OR IGNORE INTO occurrence_keywords SELECT o.mailbox_id,o.message_id,k.keyword
            FROM occurrences o JOIN sync_keywords k ON k.uid=o.uid WHERE o.mailbox_id=?1 AND o.epoch=?2
            AND NOT EXISTS(SELECT 1 FROM keyword_operations f WHERE f.mailbox_id=o.mailbox_id AND f.epoch=o.epoch AND f.uid=o.uid AND f.keyword=k.keyword)",params![mailbox,epoch])?;
        // Retired epochs/UIDs cannot receive intent. Keep a visible diagnostic.
        transaction.execute("UPDATE keyword_operations AS f SET state='blocked',detail='Source UID/UIDVALIDITY no longer exists' WHERE mailbox_id=?1
            AND NOT EXISTS(SELECT 1 FROM occurrences o WHERE o.mailbox_id=f.mailbox_id AND o.epoch=f.epoch AND o.uid=f.uid)",[mailbox])?;
        transaction.commit()?;
        Ok(epoch)
    }

    /// Keyset-page missing metadata without materializing all known UIDs. The
    /// exclusive cursor advances even for oversized or vanished messages; a new
    /// scan starts afresh and retries those not durably ingested.
    pub fn missing_sync_messages(&self, before: u64) -> Result<Vec<SyncMessage>> {
        let mut statement = self.connection.prepare_cached("SELECT s.uid,s.seen,s.flagged,s.size,s.received_at FROM sync_inventory s JOIN sync_scope c
            WHERE c.complete=1 AND s.uid<?1 AND NOT EXISTS(SELECT 1 FROM occurrences o WHERE o.mailbox_id=c.mailbox AND o.epoch=c.epoch AND o.uid=s.uid)
            ORDER BY s.uid DESC LIMIT 100")?;
        let mut messages: Vec<SyncMessage> = statement
            .query_map([before.min(u64::from(u32::MAX) + 1) as i64], |row| {
                Ok(SyncMessage {
                    uid: row.get(0)?,
                    seen: row.get(1)?,
                    starred: row.get(2)?,
                    size: row.get(3)?,
                    received_at: row.get(4)?,
                    keywords: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut keywords = self
            .connection
            .prepare_cached("SELECT keyword FROM sync_keywords WHERE uid=?1 ORDER BY keyword")?;
        for message in &mut messages {
            message.keywords = keywords
                .query_map([message.uid], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
        }
        Ok(messages)
    }
}
