use {
    crate::{Archive, Error, Result},
    rusqlite::{OptionalExtension, params},
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SyncCheckpoint {
    pub epoch: u32,
    pub upper: u32,
    pub highest_modseq: u64,
}

#[derive(Clone, Copy)]
pub enum InventoryKind {
    Full,
    /// Use SEARCH membership to reconcile deletions through the previous upper bound.
    Membership(SyncCheckpoint),
    /// Only explicitly vanished UID ranges are retired.
    Vanished(SyncCheckpoint),
}

/// Metadata only; bodies are downloaded after a complete inventory.
#[derive(Clone, Debug)]
pub struct SyncMessage {
    pub gmail: Option<GmailMetadata>,
    pub uid: u32,
    pub seen: bool,
    pub starred: bool,
    pub size: u32,
    /// IMAP INTERNALDATE as UTC Unix seconds.
    pub received_at: i64,
    pub keywords: Vec<String>,
}

/// Gmail identifiers are unsigned 64-bit values. SQLite stores their big-endian bytes.
#[derive(Clone, Debug)]
pub struct GmailMetadata {
    pub msgid: u64,
    pub thrid: u64,
    pub labels: Vec<String>,
}

pub struct SyncFlags {
    pub uid: u32,
    pub seen: bool,
    pub starred: bool,
    pub keywords: Vec<String>,
}

impl Archive {
    /// Connection-local disk-backed staging. Starting a scan discards any partial
    /// previous scan, but never changes authoritative occurrences or checkpoints.
    pub fn begin_sync_inventory(&self) -> Result<()> {
        self.connection.execute_batch("CREATE TEMP TABLE IF NOT EXISTS sync_inventory (
            uid INTEGER PRIMARY KEY, seen INTEGER NOT NULL, flagged INTEGER NOT NULL,
             size INTEGER, received_at INTEGER, gm_msgid BLOB, gm_thrid BLOB, labels TEXT);
             CREATE TEMP TABLE IF NOT EXISTS sync_keywords (uid INTEGER NOT NULL, keyword TEXT NOT NULL, PRIMARY KEY(uid,keyword));
            CREATE TEMP TABLE IF NOT EXISTS sync_scope (epoch INTEGER, upper_uid INTEGER, mailbox INTEGER, complete INTEGER, generation INTEGER, base_upper INTEGER, kind TEXT);
             CREATE TEMP TABLE IF NOT EXISTS sync_base(mailbox INTEGER, generation INTEGER);
             DELETE FROM sync_base;
             CREATE TEMP TABLE IF NOT EXISTS sync_live(uid INTEGER PRIMARY KEY);
             CREATE TEMP TABLE IF NOT EXISTS sync_vanished(lo INTEGER,hi INTEGER);
             DELETE FROM sync_live; DELETE FROM sync_vanished;
             DELETE FROM sync_inventory; DELETE FROM sync_scope; DELETE FROM sync_keywords;")?;
        Ok(())
    }

    /// Freeze the local generation before SELECT. Another worker completing
    /// an inventory invalidates this scan, even when both started without a cursor.
    pub fn begin_mailbox_inventory(&self, mailbox: i64) -> Result<Option<SyncCheckpoint>> {
        self.begin_sync_inventory()?;
        self.connection.execute(
            "INSERT INTO sync_base SELECT id,inventory_generation FROM mailboxes WHERE id=?1",
            [mailbox],
        )?;
        self.sync_checkpoint(mailbox)
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
            "INSERT INTO sync_scope SELECT ?1,?2,NULL,0,NULL,NULL,NULL WHERE NOT EXISTS(SELECT 1 FROM sync_scope)",
            params![epoch, upper_uid],
        )?;
        verify_scope(&transaction, epoch, upper_uid)?;
        {
            let mut insert = transaction
                .prepare_cached("INSERT INTO sync_inventory VALUES (?1,?2,?3,?4,?5,?6,?7,?8)")?;
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
                    message.received_at,
                    message.gmail.as_ref().map(|g| g.msgid.to_be_bytes()),
                    message.gmail.as_ref().map(|g| g.thrid.to_be_bytes()),
                    message
                        .gmail
                        .as_ref()
                        .map(|g| serde_json::to_string(&g.labels))
                        .transpose()?
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn sync_checkpoint(&self, mailbox: i64) -> Result<Option<SyncCheckpoint>> {
        Ok(self.connection.query_row(
            "SELECT epoch,inventory_upper,highest_modseq FROM mailboxes WHERE id=?1 AND highest_modseq IS NOT NULL",
            [mailbox],
            |row| Ok(SyncCheckpoint {
                epoch: row.get(0)?,
                upper: row.get(1)?,
                highest_modseq:u64::from_be_bytes(row.get(2)?),
            }),
        ).optional()?)
    }

    pub fn stage_sync_flags(&mut self, epoch: u32, upper: u32, flags: &[SyncFlags]) -> Result<()> {
        if flags.len() > 1000 || flags.iter().any(|f| f.uid == 0 || f.uid > upper) {
            return Err(Error::Invalid("Invalid staged flag updates"));
        }
        let tx = self.connection.transaction()?;
        verify_scope(&tx, epoch, upper)?;
        for item in flags {
            tx.execute(
                "INSERT INTO sync_inventory(uid,seen,flagged) VALUES (?1,?2,?3)",
                params![item.uid, item.seen, item.starred],
            )?;
            for keyword in crate::keywords::normalize(&item.keywords)? {
                tx.execute(
                    "INSERT INTO sync_keywords VALUES (?1,?2)",
                    params![item.uid, keyword],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn stage_sync_live(&mut self, epoch: u32, upper: u32, uids: &[u32]) -> Result<()> {
        if uids.len() > 1000 || uids.iter().any(|uid| *uid == 0 || *uid > upper) {
            return Err(Error::Invalid("Invalid staged live UIDs"));
        }
        let tx = self.connection.transaction()?;
        {
            verify_scope(&tx, epoch, upper)?;
            let mut insert = tx.prepare_cached("INSERT INTO sync_live VALUES (?1)")?;
            for uid in uids {
                insert.execute([uid])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    pub fn stage_sync_vanished(&self, epoch: u32, upper: u32, lo: u32, hi: u32) -> Result<()> {
        verify_scope(&self.connection, epoch, upper)?;
        if lo == 0 || hi < lo || hi > upper {
            return Err(Error::Invalid("Invalid vanished UID range"));
        }
        self.connection
            .execute("INSERT INTO sync_vanished VALUES (?1,?2)", params![lo, hi])?;
        Ok(())
    }

    /// Publish only when every selected UID has a durable occurrence and no
    /// later inventory has replaced this connection's completed scan.
    pub fn publish_sync_checkpoint(&self, mailbox: i64, modseq: u64) -> Result<()> {
        self.connection.execute("UPDATE mailboxes AS m SET highest_modseq=?2, inventory_upper=c.upper_uid FROM sync_scope c
            WHERE m.id=?1 AND c.mailbox=m.id AND c.complete=1 AND c.epoch=m.epoch AND c.generation=m.inventory_generation
            AND NOT EXISTS(SELECT 1 FROM sync_inventory s WHERE s.size IS NOT NULL AND NOT EXISTS(SELECT 1 FROM occurrences o
                WHERE o.mailbox_id=m.id AND o.epoch=m.epoch AND o.uid=s.uid))",params![mailbox,modseq.to_be_bytes()])?;
        Ok(())
    }

    /// Call only after the adapter has successfully completed the entire scan.
    /// Apply epoch, expunges and remote flags atomically; pending local flag intent
    /// wins. Bytes remain archived. Newer UIDs outside the scan are not retired.
    pub fn finish_sync_inventory(&mut self, mailbox: i64) -> Result<u32> {
        self.merge_sync_inventory(mailbox, InventoryKind::Full)
    }

    pub fn merge_sync_inventory(&mut self, mailbox: i64, kind: InventoryKind) -> Result<u32> {
        let transaction = self.connection.transaction()?;
        let (epoch, upper, complete): (u32, u32, bool) = transaction.query_row(
            "SELECT epoch,upper_uid,complete FROM sync_scope",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if complete {
            return Err(Error::Invalid("Sync inventory already applied"));
        }
        let superseded: bool = transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_base b JOIN mailboxes m ON m.id=b.mailbox
              WHERE b.mailbox<>?1 OR b.generation<>m.inventory_generation)",
            [mailbox],
            |row| row.get(0),
        )?;
        if superseded {
            return Err(Error::Invalid("Mailbox inventory was superseded"));
        }
        match kind {
            InventoryKind::Full => {
                transaction.execute("DELETE FROM occurrences WHERE mailbox_id=?1 AND (epoch<>?2 OR (uid<=?3 AND uid NOT IN (SELECT uid FROM sync_inventory)))",params![mailbox,epoch,upper])?;
            },
            InventoryKind::Membership(base) | InventoryKind::Vanished(base) => {
                let valid: bool = transaction.query_row(
                    "SELECT EXISTS(SELECT 1 FROM mailboxes WHERE id=?1 AND epoch=?2 AND highest_modseq=?3 AND inventory_upper=?4)",
                    params![mailbox,epoch,base.highest_modseq.to_be_bytes(),base.upper],
                    |row| row.get(0),
                )?;
                if !valid || base.epoch != epoch || base.upper > upper {
                    return Err(Error::Invalid("Delta inventory checkpoint changed"));
                }
                if matches!(kind, InventoryKind::Membership(_)) {
                    transaction.execute("DELETE FROM occurrences WHERE mailbox_id=?1 AND epoch=?2 AND uid<=?3 AND uid NOT IN (SELECT uid FROM sync_live)",params![mailbox,epoch,base.upper])?;
                } else {
                    let mut ranges = transaction.prepare("SELECT lo,hi FROM sync_vanished")?;
                    for range in
                        ranges.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))?
                    {
                        let (lo, hi) = range?;
                        transaction.execute("DELETE FROM occurrences WHERE mailbox_id=?1 AND epoch=?2 AND uid BETWEEN ?3 AND ?4",params![mailbox,epoch,lo,hi.min(base.upper)])?;
                    }
                }
            },
        }
        match kind {
            InventoryKind::Full => {
                transaction.execute(
                    "DELETE FROM deletion_inventory WHERE mailbox_id=?1 AND (epoch<>?2 OR uid<=?3)",
                    params![mailbox, epoch, upper],
                )?;
            },
            InventoryKind::Membership(base) => {
                transaction.execute("DELETE FROM deletion_inventory WHERE mailbox_id=?1 AND epoch=?2 AND uid<=?3 AND uid NOT IN (SELECT uid FROM sync_live)",params![mailbox,epoch,base.upper])?;
            },
            InventoryKind::Vanished(_) => {
                transaction.execute("DELETE FROM deletion_inventory WHERE mailbox_id=?1 AND epoch=?2 AND EXISTS(SELECT 1 FROM sync_vanished v WHERE uid BETWEEN v.lo AND v.hi)",params![mailbox,epoch])?;
            },
        }
        transaction.execute("INSERT OR IGNORE INTO deletion_inventory SELECT m.id,?2,s.uid FROM mailboxes m CROSS JOIN sync_inventory s WHERE m.id=?1 AND COALESCE(m.user_role,m.server_role) IN ('trash','junk')",params![mailbox,epoch])?;
        if transaction.execute(
            "UPDATE mailboxes SET epoch=?2, highest_modseq=NULL,inventory_generation=inventory_generation+1 WHERE id=?1",
            params![mailbox, epoch],
        )? != 1
        {
            return Err(Error::NotFound);
        }
        transaction.execute("UPDATE occurrences AS o SET
            seen=CASE WHEN EXISTS(SELECT 1 FROM flag_operations f WHERE f.mailbox_id=o.mailbox_id AND f.epoch=o.epoch AND f.uid=o.uid AND f.kind='seen') THEN seen ELSE (SELECT seen FROM sync_inventory s WHERE s.uid=o.uid) END,
            flagged=CASE WHEN EXISTS(SELECT 1 FROM flag_operations f WHERE f.mailbox_id=o.mailbox_id AND f.epoch=o.epoch AND f.uid=o.uid AND f.kind='flagged') THEN flagged ELSE (SELECT flagged FROM sync_inventory s WHERE s.uid=o.uid) END
            WHERE mailbox_id=?1 AND epoch=?2 AND uid IN (SELECT uid FROM sync_inventory)",params![mailbox,epoch])?;
        if transaction.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_inventory WHERE gm_msgid IS NOT NULL)",
            [],
            |row| row.get(0),
        )? {
            crate::gmail::reconcile(&transaction, mailbox, epoch)?;
        }
        let (base_upper, mode) = match kind {
            InventoryKind::Full => (0, "full"),
            InventoryKind::Membership(base) => (base.upper, "membership"),
            InventoryKind::Vanished(base) => (base.upper, "vanished"),
        };
        transaction.execute("UPDATE sync_scope SET mailbox=?1,complete=1,generation=(SELECT inventory_generation FROM mailboxes WHERE id=?1),base_upper=?2,kind=?3", params![mailbox,base_upper,mode])?;
        transaction.execute("DELETE FROM occurrence_keywords AS k WHERE k.mailbox_id=?1
            AND EXISTS(SELECT 1 FROM occurrences o JOIN sync_inventory s ON s.uid=o.uid WHERE o.mailbox_id=k.mailbox_id AND o.message_id=k.message_id AND o.epoch=?2)
            AND NOT EXISTS(SELECT 1 FROM keyword_operations f JOIN occurrences o ON o.mailbox_id=f.mailbox_id AND o.epoch=f.epoch AND o.uid=f.uid
                WHERE o.mailbox_id=k.mailbox_id AND o.message_id=k.message_id AND f.gmail=0 AND f.keyword=k.keyword)",params![mailbox,epoch])?;
        transaction.execute("INSERT OR IGNORE INTO occurrence_keywords SELECT o.mailbox_id,o.message_id,k.keyword
            FROM occurrences o JOIN sync_keywords k ON k.uid=o.uid WHERE o.mailbox_id=?1 AND o.epoch=?2
            AND NOT EXISTS(SELECT 1 FROM keyword_operations f WHERE f.mailbox_id=o.mailbox_id AND f.epoch=o.epoch AND f.uid=o.uid AND f.gmail=0 AND f.keyword=k.keyword)",params![mailbox,epoch])?;
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
        let mut statement = self.connection.prepare_cached("SELECT s.uid,s.seen,s.flagged,s.size,s.received_at,s.gm_msgid,s.gm_thrid,s.labels FROM sync_inventory s JOIN sync_scope c
            WHERE c.complete=1 AND s.size IS NOT NULL AND s.uid<?1 AND NOT EXISTS(SELECT 1 FROM occurrences o WHERE o.mailbox_id=c.mailbox AND o.epoch=c.epoch AND o.uid=s.uid)
            ORDER BY s.uid DESC LIMIT 100")?;
        let mut messages: Vec<SyncMessage> = statement
            .query_map([before.min(u64::from(u32::MAX) + 1) as i64], |row| {
                Ok(SyncMessage {
                    gmail: match row.get::<_, Option<[u8; 8]>>(5)? {
                        Some(bytes) => Some(GmailMetadata {
                            msgid: u64::from_be_bytes(bytes),
                            thrid: u64::from_be_bytes(row.get(6)?),
                            labels: serde_json::from_str(&row.get::<_, String>(7)?).map_err(
                                |e| {
                                    rusqlite::Error::FromSqlConversionFailure(
                                        7,
                                        rusqlite::types::Type::Text,
                                        Box::new(e),
                                    )
                                },
                            )?,
                        }),
                        None => None,
                    },
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

fn verify_scope(db: &rusqlite::Connection, epoch: u32, upper_uid: u32) -> Result<()> {
    let matches: bool = db.query_row(
        "SELECT epoch=?1 AND upper_uid=?2 AND complete=0 FROM sync_scope",
        params![epoch, upper_uid],
        |row| row.get(0),
    )?;
    if !matches {
        return Err(Error::Invalid("Sync inventory scope changed"));
    }
    Ok(())
}
