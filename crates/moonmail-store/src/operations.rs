use {
    crate::{Archive, Error, Result},
    moonmail_core::MessageId,
    rusqlite::{OptionalExtension, params},
    serde::{Deserialize, Serialize},
};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Draft {
    pub id: String,
    pub recipients: String,
    pub subject: String,
    pub body: String,
    pub reply_id: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct OutboxItem {
    pub id: i64,
    pub account: String,
    pub blob: String,
    pub sender: String,
    pub recipients: Vec<String>,
    pub subject: String,
    pub state: String,
    pub detail: String,
}

pub struct PendingFlag {
    pub id: i64,
    pub mailbox: String,
    pub epoch: u32,
    pub uid: u32,
    pub kind: String,
    pub value: bool,
}

impl Archive {
    pub fn set_flag(&mut self, id: MessageId, kind: &str, value: bool) -> Result<()> {
        let column = match kind {
            "seen" => "seen",
            "flagged" => "flagged",
            _ => return Err(Error::Invalid("unknown flag")),
        };
        let transaction = self.connection.transaction()?;
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

    pub fn save_draft(&self, draft: &Draft) -> Result<()> {
        self.connection.execute("INSERT INTO drafts VALUES (?1,?2,?3,?4,?5,unixepoch()) ON CONFLICT(id) DO UPDATE SET recipients=excluded.recipients,subject=excluded.subject,body=excluded.body,reply_id=excluded.reply_id,updated=excluded.updated",params![draft.id,draft.recipients,draft.subject,draft.body,draft.reply_id])?;
        Ok(())
    }

    pub fn latest_draft(&self) -> Result<Option<Draft>> {
        Ok(self.connection.query_row("SELECT id,recipients,subject,body,reply_id FROM drafts ORDER BY updated DESC LIMIT 1",[],|row|Ok(Draft{id:row.get(0)?,recipients:row.get(1)?,subject:row.get(2)?,body:row.get(3)?,reply_id:row.get(4)?})).optional()?)
    }

    pub fn queue(
        &mut self,
        draft: &Draft,
        account: &str,
        blob: &str,
        recipients: &[String],
    ) -> Result<i64> {
        let transaction = self.connection.transaction()?;
        transaction.execute("INSERT INTO outbox(draft_id,account,blob,sender,recipients,subject,state,created) VALUES (?1,?2,?3,?2,?4,?5,'queued',unixepoch()) ON CONFLICT(draft_id) DO NOTHING",params![draft.id,account,blob,serde_json::to_string(recipients)?,draft.subject])?;
        let id = transaction.query_row(
            "SELECT id FROM outbox WHERE draft_id=?1",
            [&draft.id],
            |row| row.get(0),
        )?;
        transaction.execute("DELETE FROM drafts WHERE id=?1", [&draft.id])?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn outbox(&self) -> Result<Vec<OutboxItem>> {
        let mut statement = self.connection.prepare("SELECT id,account,blob,sender,recipients,subject,state,detail FROM outbox ORDER BY id DESC LIMIT 100")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        })?;
        rows.map(|row| {
            let (id, account, blob, sender, recipients, subject, state, detail) = row?;
            Ok(OutboxItem {
                id,
                account,
                blob,
                sender,
                recipients: serde_json::from_str(&recipients)?,
                subject,
                state,
                detail,
            })
        })
        .collect()
    }

    pub fn claim_send(&self, id: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE outbox SET state='sending',detail='' WHERE id=?1 AND state='queued'",
            [id],
        )? == 1)
    }

    pub fn send_result(&self, id: i64, state: &str, detail: &str) -> Result<()> {
        if !matches!(state, "sent" | "failed" | "unknown" | "queued") {
            return Err(Error::Invalid("invalid outbox state"));
        }
        self.connection.execute(
            "UPDATE outbox SET state=?2,detail=?3 WHERE id=?1 AND state='sending'",
            params![id, state, detail],
        )?;
        Ok(())
    }

    /// Call only while holding the process-wide application lock at startup.
    pub fn recover_interrupted_sends(&self) -> Result<()> {
        self.connection.execute("UPDATE outbox SET state='unknown',detail='Application stopped during submission. Check Sent before composing a replacement.' WHERE state='sending'",[])?;
        Ok(())
    }
}
