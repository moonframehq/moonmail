use {
    crate::{Archive, Error, Result},
    moonmail_core::{MailboxRole, MessageId},
    rusqlite::{OptionalExtension, params},
    serde::Serialize,
};

#[derive(Clone, Copy)]
pub enum MoveTarget {
    Folder(i64),
    Role(MailboxRole),
}

#[derive(Clone, Debug, Serialize)]
pub struct MoveOperation {
    /// "move" removes the source after acknowledgment; "copy" always retains it.
    pub kind: String,
    pub id: i64,
    pub message_id: MessageId,
    pub subject: String,
    pub account: String,
    pub source_id: i64,
    pub source: String,
    pub destination_id: i64,
    pub destination: String,
    pub epoch: u32,
    pub uid: u32,
    pub state: String,
    pub detail: String,
}

const SELECT: &str = "SELECT o.id,o.message_id,m.subject,s.account,o.source_id,s.name,o.destination_id,d.name,o.epoch,o.uid,o.state,o.detail,o.kind
    FROM move_operations o JOIN messages m ON m.id=o.message_id
    JOIN mailboxes s ON s.id=o.source_id JOIN mailboxes d ON d.id=o.destination_id";

impl Archive {
    /// Queue up to 100 explicit messages atomically, preserving the source's
    /// UIDVALIDITY/UID. A grouped row means its representative message only.
    /// No optimistic remote occurrence changes: pending moves stay visible.
    /// Local (UID-less) messages move within their account in this transaction.
    pub fn queue_moves(
        &mut self,
        ids: &[MessageId],
        source: Option<i64>,
        target: MoveTarget,
    ) -> Result<usize> {
        self.queue_transfers(ids, source, target, false)
    }

    /// Same source/account and durable-outcome rules as moves. Copies retain
    /// source occurrences; local copies preserve an already existing destination.
    pub fn queue_copies(
        &mut self,
        ids: &[MessageId],
        source: Option<i64>,
        target: MoveTarget,
    ) -> Result<usize> {
        self.queue_transfers(ids, source, target, true)
    }

    fn queue_transfers(
        &mut self,
        ids: &[MessageId],
        source: Option<i64>,
        target: MoveTarget,
        copy: bool,
    ) -> Result<usize> {
        if ids.is_empty() || ids.len() > 100 {
            return Err(Error::Invalid("Choose between one and 100 messages"));
        }
        // Resolve roles before the transaction; revalidate destination below.
        let mut resolved = Vec::new();
        for id in ids {
            if resolved.iter().any(|(previous, _)| previous == id) {
                continue;
            }
            let account: String = self.connection.query_row(
                "SELECT account FROM messages WHERE id=?1",
                [id.0 as i64],
                |row| row.get(0),
            )?;
            let destination = match target {
                MoveTarget::Folder(id) => id,
                MoveTarget::Role(role) => self.role_mailbox(&account, role)?,
            };
            resolved.push((*id, destination));
        }
        let transaction = self.connection.transaction()?;
        let mut count = 0;
        for (id, destination) in resolved {
            let mut statement = transaction.prepare("SELECT o.mailbox_id,o.epoch,o.uid,o.seen,o.flagged,b.account FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id WHERE o.message_id=?1 AND b.active=1 AND b.selectable=1 AND (?2 IS NULL OR o.mailbox_id=?2) LIMIT 2")?;
            let origins = statement
                .query_map(params![id.0 as i64, source], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, u32>(1)?,
                        row.get::<_, Option<u32>>(2)?,
                        row.get::<_, bool>(3)?,
                        row.get::<_, bool>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            if origins.len() != 1 {
                return Err(Error::Invalid(
                    "Choose the message's source folder before moving it",
                ));
            }
            let (source, epoch, uid, seen, flagged, account) = &origins[0];
            let editing:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM folder_operations WHERE account=?1 AND state IN ('queued','running','unknown'))",[account],|row|row.get(0))?;
            if editing {
                return Err(Error::Invalid(
                    "Finish, cancel, or review the pending folder operation before moving messages",
                ));
            }
            let valid: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM mailboxes WHERE id=?1 AND account=?2 AND active=1 AND selectable=1)", params![destination,account], |row| row.get(0))?;
            if !valid {
                return Err(Error::Invalid(
                    "Move destination must be an active folder in the same account",
                ));
            }
            if *source == destination {
                continue;
            }
            let pending: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM move_operations WHERE message_id=?1 AND state IN ('queued','running','unknown'))", [id.0 as i64], |row| row.get(0))?;
            if pending {
                return Err(Error::Invalid(
                    "Message already has a pending or uncertain move; inspect Mail operations",
                ));
            }
            if let Some(uid) = uid {
                transaction.execute("INSERT INTO move_operations(message_id,source_id,destination_id,epoch,uid,state,kind) VALUES (?1,?2,?3,?4,?5,'queued',?6)", params![id.0 as i64,source,destination,epoch,uid,if copy {"copy"}else{"move"}])?;
            } else {
                let present:bool=transaction.query_row("SELECT EXISTS(SELECT 1 FROM occurrences WHERE mailbox_id=?1 AND message_id=?2)",params![destination,id.0 as i64],|r|r.get(0))?;
                let conflict = if copy {
                    "DO NOTHING"
                } else {
                    "DO UPDATE SET seen=excluded.seen,flagged=excluded.flagged"
                };
                let changed=transaction.execute(&format!("INSERT INTO occurrences(mailbox_id,message_id,epoch,uid,seen,flagged) VALUES (?1,?2,0,NULL,?3,?4) ON CONFLICT(mailbox_id,message_id) {conflict}"), params![destination,id.0 as i64,seen,flagged])?;
                if changed > 0 {
                    transaction.execute(
                        "DELETE FROM occurrence_keywords WHERE mailbox_id=?1 AND message_id=?2",
                        params![destination, id.0 as i64],
                    )?;
                    transaction.execute("INSERT INTO occurrence_keywords SELECT ?1,message_id,keyword FROM occurrence_keywords WHERE mailbox_id=?2 AND message_id=?3",params![destination,source,id.0 as i64])?;
                }
                if !copy {
                    transaction.execute(
                        "DELETE FROM occurrences WHERE mailbox_id=?1 AND message_id=?2",
                        params![source, id.0 as i64],
                    )?;
                }
                if !present {
                    crate::rules::on_entry(&transaction, destination, id)?;
                }
            }
            count += 1;
        }
        transaction.commit()?;
        Ok(count)
    }

    pub fn has_pending_move(&self, id: MessageId) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM move_operations WHERE message_id=?1 AND state IN ('queued','running','unknown'))", [id.0 as i64], |row| row.get(0))?)
    }

    pub fn unresolved_move_count(&self) -> Result<u64> {
        Ok(self.connection.query_row("SELECT COUNT(*) FROM move_operations WHERE state IN ('queued','running','unknown','failed')", [], |row|row.get::<_,i64>(0))? as u64)
    }

    /// Bounded UI history; unresolved work sorts ahead of completed history.
    pub fn move_operations(&self) -> Result<Vec<MoveOperation>> {
        self.read_moves(&format!("{SELECT} ORDER BY (o.state IN ('queued','running','unknown','failed')) DESC,o.id DESC LIMIT 100"), None)
    }

    /// A sync processes one bounded batch; additional queued work remains durable.
    pub fn pending_moves(&self, account: &str) -> Result<Vec<MoveOperation>> {
        self.read_moves(
            &format!("{SELECT} WHERE s.account=?1 AND o.state='queued' ORDER BY o.id LIMIT 100"),
            Some(account),
        )
    }

    fn read_moves(&self, sql: &str, account: Option<&str>) -> Result<Vec<MoveOperation>> {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(rusqlite::params_from_iter(account), |row| {
            Ok(MoveOperation {
                kind: row.get(12)?,
                id: row.get(0)?,
                message_id: MessageId(row.get::<_, i64>(1)? as u64),
                subject: row.get(2)?,
                account: row.get(3)?,
                source_id: row.get(4)?,
                source: row.get(5)?,
                destination_id: row.get(6)?,
                destination: row.get(7)?,
                epoch: row.get(8)?,
                uid: row.get(9)?,
                state: row.get(10)?,
                detail: row.get(11)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Claim only immediately before remote dispatch. Cancellation wins if it
    /// commits first. A lost response or crash after this point is never retried.
    pub fn claim_move(&self, id: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE move_operations SET state='running',detail='' WHERE id=?1 AND state='queued'",
            [id],
        )? == 1)
    }

    pub fn fail_queued_move(&self, id: i64, detail: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE move_operations SET state='failed',detail=?2 WHERE id=?1 AND state='queued'",
            params![id, detail],
        )?;
        Ok(())
    }

    /// Record copy proof before any source deletion. A crash still recovers as
    /// unknown; this receipt is diagnostic evidence, never permission to retry.
    pub fn record_move_copy(&self, id: i64, epoch: u32, uid: u32) -> Result<()> {
        if epoch == 0 || uid == 0 {
            return Err(Error::Invalid("Invalid destination copy receipt"));
        }
        if self.connection.execute(
            "UPDATE move_operations SET detail=?2 WHERE id=?1 AND state='running'",
            params![
                id,
                format!("COPY acknowledged destination UIDVALIDITY {epoch}, UID {uid}. ")
            ],
        )? != 1
        {
            return Err(Error::Invalid("Move is no longer running"));
        }
        Ok(())
    }

    /// Acknowledged remote moves retire only the exact original occurrence.
    /// Destination mail is downloaded during normal sync; immutable bytes remain.
    pub fn finish_move(&mut self, id: i64, state: &str, detail: &str) -> Result<()> {
        if !matches!(state, "done" | "unknown" | "failed") {
            return Err(Error::Invalid("invalid move result"));
        }
        let transaction = self.connection.transaction()?;
        let operation = transaction.query_row("SELECT source_id,epoch,uid,message_id,kind FROM move_operations WHERE id=?1 AND state='running'", [id], |row| Ok((row.get::<_,i64>(0)?,row.get::<_,u32>(1)?,row.get::<_,u32>(2)?,row.get::<_,i64>(3)?,row.get::<_,String>(4)?))).optional()?;
        if let Some((source, epoch, uid, message, kind)) = operation {
            if state == "done" && kind == "move" {
                transaction.execute("DELETE FROM occurrences WHERE mailbox_id=?1 AND epoch=?2 AND uid=?3 AND message_id=?4", params![source,epoch,uid,message])?;
                transaction.execute(
                    "DELETE FROM flag_operations WHERE mailbox_id=?1 AND epoch=?2 AND uid=?3",
                    params![source, epoch, uid],
                )?;
            }
            transaction.execute(
                "UPDATE move_operations SET state=?2,detail=detail || ?3 WHERE id=?1 AND state='running'",
                params![id, state, detail],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn cancel_move(&self, id: i64) -> Result<()> {
        if self.connection.execute("UPDATE move_operations SET state='cancelled',detail='Cancelled before dispatch' WHERE id=?1 AND state IN ('queued','failed')", [id])? == 0 {
            return Err(Error::Invalid("Only queued or failed moves can be cancelled"));
        }
        Ok(())
    }

    pub fn dismiss_unknown_move(&self, id: i64) -> Result<()> {
        if self.connection.execute("UPDATE move_operations SET state='dismissed',detail='Uncertain result reviewed by user; no retry was sent' WHERE id=?1 AND state='unknown'", [id])? == 0 {
            return Err(Error::Invalid("Move is not awaiting outcome review"));
        }
        Ok(())
    }

    /// Startup only, under the archive's application lock; never on worker open.
    pub fn recover_interrupted_moves(&self) -> Result<()> {
        self.connection.execute("UPDATE move_operations SET state='unknown',detail=detail || 'Interrupted during move. Sync and inspect source/destination before dismissing.' WHERE state='running'", [])?;
        Ok(())
    }
}
