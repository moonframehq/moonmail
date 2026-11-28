use {
    crate::{Archive, Error, Result},
    moonmail_core::{MailboxRole, MessageId},
    rusqlite::params,
};

#[derive(serde::Serialize)]
pub struct RemoteOperation {
    pub id: i64,
    pub account: String,
    pub mailbox: String,
    pub epoch: u32,
    pub uid: u32,
    pub state: String,
    pub kind: String,
    pub message_id: Option<String>,
    pub subject: Option<String>,
}

pub enum DeletionTarget {
    Message(MessageId, u32),
    Through(u32),
}

impl Archive {
    pub fn deletion_bounds(
        &self,
        mailbox: i64,
        epoch: u32,
        message: Option<MessageId>,
    ) -> Result<(u32, Option<u32>)> {
        if epoch == 0 {
            return Ok(self.connection.query_row(
                "SELECT coalesce((SELECT revision FROM jmap_deletion_revisions WHERE mailbox=?1),0),
                 (SELECT uid FROM occurrences WHERE mailbox_id=?1 AND epoch=0 AND message_id=?2)
                 FROM jmap_folders WHERE mailbox=?1",
                params![mailbox,message.map(|m|m.0 as i64)], |r|Ok((r.get(0)?,r.get(1)?))
            )?);
        }
        Ok(self.connection.query_row(
            "SELECT MAX(
                COALESCE((SELECT MAX(uid) FROM deletion_inventory WHERE mailbox_id=?1 AND epoch=?2),0),
                COALESCE((SELECT MAX(uid) FROM occurrences WHERE mailbox_id=?1 AND epoch=?2),0)),
                (SELECT uid FROM occurrences WHERE mailbox_id=?1 AND epoch=?2 AND message_id=?3)",
            params![mailbox, epoch, message.map(|m|m.0 as i64)],
            |row| Ok((row.get(0)?,row.get(1)?)),
        )?)
    }

    /// Freeze only UIDs from the confirmed local snapshot. Messages arriving
    /// after this transaction can never be consumed by the queued operation.
    pub fn queue_expunge(
        &mut self,
        account: &str,
        mailbox: i64,
        name: &str,
        epoch: u32,
        role: MailboxRole,
        target: DeletionTarget,
    ) -> Result<usize> {
        if !matches!(role, MailboxRole::Trash | MailboxRole::Junk)
            || self.role_mailbox(account, role)? != mailbox
        {
            return Err(Error::Invalid("Choose the current Trash or Junk folder"));
        }
        let tx = self.connection.transaction()?;
        let valid: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM mailboxes WHERE id=?1 AND account=?2 AND name=?3
                AND epoch=?4 AND active AND selectable AND COALESCE(user_role,server_role)=?5)",
            params![mailbox, account, name, epoch, role.as_str()],
            |r| r.get(0),
        )?;
        if !valid {
            return Err(Error::Invalid("Folder changed; reopen confirmation"));
        }
        if epoch == 0 {
            let (folder, revision): (String,u32) = tx.query_row(
                "SELECT f.remote,coalesce(r.revision,0) FROM jmap_folders f
                 LEFT JOIN jmap_deletion_revisions r ON r.mailbox=f.mailbox WHERE f.account=?1 AND f.mailbox=?2",
                params![account,mailbox], |r|Ok((r.get(0)?,r.get(1)?))
            )?;
            let message = match target {
                DeletionTarget::Through(expected) if expected == revision => None,
                DeletionTarget::Message(message,uid) if tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM occurrences WHERE mailbox_id=?1 AND message_id=?2 AND epoch=0 AND uid=?3)",
                    params![mailbox,message.0 as i64,uid], |r|r.get::<_,bool>(0)
                )? => Some(message.0 as i64),
                _ => return Err(Error::Invalid("Folder changed; reopen confirmation")),
            };
            let count = tx.execute(
                "INSERT INTO expunge_operations(account,mailbox,epoch,uid,remote_folder,confirmed_role)
                 SELECT ?1,?2,0,o.uid,?3,?4 FROM occurrences o JOIN jmap_emails e ON e.account=?1 AND e.uid=o.uid
                 WHERE o.mailbox_id=?5 AND o.epoch=0 AND (?6 IS NULL OR o.message_id=?6)
                 ON CONFLICT(account,remote_folder,uid) WHERE epoch=0 DO UPDATE SET state='queued',
                 mailbox=excluded.mailbox,confirmed_role=excluded.confirmed_role
                 WHERE state IN ('done','dismissed','stale')",
                params![account,name,folder,role.as_str(),mailbox,message],
            )?;
            tx.commit()?;
            return Ok(count);
        }
        let count = match target {
            DeletionTarget::Message(id, expected) => {
                let uid: u32 = tx.query_row(
                    "SELECT uid FROM occurrences WHERE mailbox_id=?1 AND epoch=?2 AND message_id=?3",
                    params![mailbox,epoch,id.0 as i64], |r|r.get(0)
                )?;
                if uid != expected {
                    return Err(Error::Invalid("Message changed; reopen confirmation"));
                }
                tx.execute(
                    "INSERT OR IGNORE INTO expunge_operations(account,mailbox,epoch,uid)
                     VALUES(?1,?2,?3,?4)",
                    params![account,name,epoch,uid],
                )?
            }
            DeletionTarget::Through(upper) => {
                tx.execute(
                    "INSERT OR IGNORE INTO expunge_operations(account,mailbox,epoch,uid)
                     SELECT ?1,?2,?3,uid FROM deletion_inventory WHERE mailbox_id=?4 AND epoch=?3 AND uid<=?5
                     UNION SELECT ?1,?2,?3,uid FROM occurrences WHERE mailbox_id=?4 AND epoch=?3 AND uid<=?5",
                    params![account,name,epoch,mailbox,upper]
                )?
            }
        };
        tx.commit()?;
        Ok(count)
    }
    pub fn remote_operations(&self, account: Option<&str>) -> Result<Vec<RemoteOperation>> {
        let sql = if account.is_some() {
            "SELECT id,account,mailbox,epoch,uid,state,'Delete',NULL,NULL
             FROM expunge_operations WHERE account=?1 AND epoch>0 AND state='queued' ORDER BY id LIMIT 100"
        } else {
            "SELECT * FROM (
                 SELECT id,account,mailbox,epoch,uid,state,'Delete',NULL,NULL FROM expunge_operations
                 UNION ALL SELECT rowid,account,mailbox,0,0,state,'Draft upload',message_id,subject
                 FROM draft_uploads)
             WHERE ?1 IS NULL ORDER BY state IN ('done','dismissed'),id DESC LIMIT 100"
        };
        let mut q = self.connection.prepare(sql)?;
        Ok(q.query_map([account], |r| {
            Ok(RemoteOperation {
                id: r.get(0)?,
                account: r.get(1)?,
                mailbox: r.get(2)?,
                epoch: r.get(3)?,
                uid: r.get(4)?,
                state: r.get(5)?,
                kind: r.get(6)?,
                message_id: r.get(7)?,
                subject: r.get(8)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
    pub fn unresolved_remote_count(&self) -> Result<u64> {
        Ok(self.connection.query_row("SELECT (SELECT COUNT(*) FROM expunge_operations WHERE state NOT IN ('done','dismissed'))+(SELECT COUNT(*) FROM draft_uploads)",[],|r|r.get::<_,i64>(0))? as u64)
    }
    pub fn claim_expunge(&self, id: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE expunge_operations SET state='unknown' WHERE id=?1 AND state='queued'",
            [id],
        )? == 1)
    }
    pub fn finish_expunge(&mut self, id: i64, state: &str) -> Result<()> {
        if !matches!(
            state,
            "done" | "unsupported" | "stale" | "queued" | "dismissed"
        ) {
            return Err(Error::Invalid("Invalid deletion state"));
        }
        let tx = self.connection.transaction()?;
        tx.execute("UPDATE expunge_operations SET state=?2 WHERE id=?1 AND state IN ('unknown','stale','unsupported')",params![id,state])?;
        if state == "done" {
            tx.execute("DELETE FROM deletion_inventory WHERE (mailbox_id,epoch,uid)=(SELECT m.id,e.epoch,e.uid FROM expunge_operations e JOIN mailboxes m ON m.account=e.account AND m.name=e.mailbox WHERE e.id=?1 AND e.state='done' AND e.epoch>0)",[id])?;
            tx.execute("DELETE FROM occurrences WHERE (mailbox_id,epoch,uid)=(SELECT m.id,e.epoch,e.uid FROM expunge_operations e JOIN mailboxes m ON m.account=e.account AND m.name=e.mailbox WHERE e.id=?1 AND e.state='done' AND e.epoch>0)",[id])?;
        }
        if matches!(state, "done" | "dismissed") {
            tx.execute("UPDATE drafts SET pending_remote_delete=0 WHERE id=(SELECT draft_id FROM expunge_operations WHERE id=?1) AND NOT EXISTS(SELECT 1 FROM expunge_operations WHERE draft_id=drafts.id AND state NOT IN ('done','dismissed'))",[id])?;
        }
        tx.commit()?;
        Ok(())
    }
}
