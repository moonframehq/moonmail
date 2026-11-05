use {
    crate::{Archive, Draft, Error, Result},
    rusqlite::{OptionalExtension, TransactionBehavior, params},
    serde::Serialize,
};

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
    pub editable: bool,
    pub sent_status: String,
    pub sent_message: Option<moonmail_core::MessageId>,
    pub sent_copy_state: String,
    pub sent_copy_detail: String,
}

pub struct OutboxPage {
    pub rows: Vec<OutboxItem>,
    pub next: Option<i64>,
}

const SELECT: &str = "SELECT id,account,blob,sender,recipients,subject,state,detail,draft_json IS NOT NULL,sent_status,sent_message,sent_copy_state,sent_copy_detail FROM outbox";

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(OutboxItem, String)> {
    Ok((
        OutboxItem {
            id: row.get(0)?,
            account: row.get(1)?,
            blob: row.get(2)?,
            sender: row.get(3)?,
            recipients: Vec::new(),
            subject: row.get(5)?,
            state: row.get(6)?,
            detail: row.get(7)?,
            editable: row.get(8)?,
            sent_status: row.get(9)?,
            sent_message: row
                .get::<_, Option<i64>>(10)?
                .map(|id| moonmail_core::MessageId(id as u64)),
            sent_copy_state: row.get(11)?,
            sent_copy_detail: row.get(12)?,
        },
        row.get(4)?,
    ))
}

fn decode((mut item, recipients): (OutboxItem, String)) -> Result<OutboxItem> {
    item.recipients = serde_json::from_str(&recipients)?;
    item.editable &= matches!(item.state.as_str(), "queued" | "failed");
    Ok(item)
}

impl Archive {
    pub fn queue(
        &mut self,
        draft: &Draft,
        account: &str,
        blob: &str,
        recipients: &[String],
    ) -> Result<i64> {
        self.queue_with_message_id(draft, account, blob, recipients, None)
    }

    /// The ID must be extracted from the frozen MIME being queued, never from
    /// an editable subject or a regenerated message. Legacy queues remain valid.
    pub fn queue_with_message_id(
        &mut self,
        draft: &Draft,
        account: &str,
        blob: &str,
        recipients: &[String],
        message_id: Option<&str>,
    ) -> Result<i64> {
        if message_id
            .is_some_and(|id| id.is_empty() || id.len() > 998 || id.chars().any(char::is_control))
        {
            return Err(Error::Invalid("Invalid frozen Message-ID"));
        }
        if draft.id.is_empty() || (!draft.account.is_empty() && draft.account != account) {
            return Err(Error::Invalid("Draft ID or account does not match"));
        }
        draft.validate_attachments()?;
        let mut saved = draft.clone();
        saved.account = account.to_owned();
        saved.validate_identity()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute("INSERT INTO outbox(draft_id,account,blob,sender,recipients,subject,state,created,draft_json,rfc_id,sent_status) VALUES (?1,?2,?3,?8,?4,?5,'queued',unixepoch(),?6,?7,CASE WHEN ?7 IS NULL THEN 'untracked' ELSE 'pending' END) ON CONFLICT(draft_id) DO NOTHING", params![draft.id, account, blob, serde_json::to_string(recipients)?, draft.subject, serde_json::to_string(&saved)?,message_id,saved.sender()])?;
        let id = transaction.query_row(
            "SELECT id FROM outbox WHERE draft_id=?1",
            [&draft.id],
            |row| row.get(0),
        )?;
        transaction.execute("DELETE FROM drafts WHERE id=?1", [&draft.id])?;
        transaction.commit()?;
        Ok(id)
    }

    /// Exact lookup must not be limited by the currently visible history page.
    pub fn outbox_item(&self, id: i64) -> Result<OutboxItem> {
        let item = self
            .connection
            .query_row(&format!("{SELECT} WHERE id=?1"), [id], row)
            .optional()?
            .ok_or(Error::NotFound)?;
        decode(item)
    }

    pub fn outbox(&self) -> Result<Vec<OutboxItem>> {
        Ok(self.outbox_page(None, 100)?.rows)
    }

    /// Metadata only; the recovery snapshot is read solely when editing.
    pub fn outbox_page(&self, before: Option<i64>, limit: usize) -> Result<OutboxPage> {
        let limit = limit.clamp(1, 100);
        let mut statement = self
            .connection
            .prepare(&format!("{SELECT} WHERE id < ?1 ORDER BY id DESC LIMIT ?2"))?;
        let mut rows = statement
            .query_map(params![before.unwrap_or(i64::MAX), (limit + 1) as i64], row)?
            .map(|row| decode(row?))
            .collect::<Result<Vec<_>>>()?;
        let more = rows.len() > limit;
        rows.truncate(limit);
        let next = if more {
            rows.last().map(|item| item.id)
        } else {
            None
        };
        Ok(OutboxPage { rows, next })
    }

    pub fn outbox_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM outbox", [], |row| {
                row.get::<_, i64>(0)
            })? as u64)
    }

    pub fn claim_send(&self, id: i64) -> Result<bool> {
        Ok(self.connection.execute(
            "UPDATE outbox SET state='sending',detail='' WHERE id=?1 AND state='queued'",
            [id],
        )? == 1)
    }

    /// `failed` means the adapter proved submission did not begin. Any ambiguity
    /// must be recorded as `unknown`; neither state is retried automatically.
    pub fn send_result(&self, id: i64, state: &str, detail: &str) -> Result<()> {
        if !matches!(state, "sent" | "failed" | "unknown") {
            return Err(Error::Invalid("invalid outbox result"));
        }
        if self.connection.execute(
            "UPDATE outbox SET state=?2,detail=?3 WHERE id=?1 AND state='sending'",
            params![id, state, detail],
        )? != 1
        {
            return Err(Error::Invalid("Outbox attempt is no longer running"));
        }
        Ok(())
    }

    pub fn retry_outbox(&self, id: i64) -> Result<()> {
        if self.connection.execute("UPDATE outbox SET state='queued',detail='Explicit retry requested' WHERE id=?1 AND state='failed'", [id])? != 1 {
            return Err(Error::Invalid("Only a known pre-submission failure can be retried"));
        }
        Ok(())
    }

    pub fn cancel_outbox(&self, id: i64) -> Result<()> {
        if self.connection.execute("UPDATE outbox SET state='cancelled',detail='Cancelled before submission' WHERE id=?1 AND state IN ('queued','failed')", [id])? != 1 {
            return Err(Error::Invalid("Only unsent queued or failed messages can be cancelled"));
        }
        Ok(())
    }

    /// Atomically cancel the original and restore its complete compose snapshot
    /// under a fresh ID. The original tombstone and frozen bytes remain immutable.
    /// Duplicate edit requests fail without making duplicate drafts.
    pub fn restore_outbox_draft(&mut self, id: i64, new_id: &str) -> Result<Draft> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (state, snapshot): (String, Option<String>) = transaction
            .query_row(
                "SELECT state,draft_json FROM outbox WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?
            .ok_or(Error::NotFound)?;
        if !matches!(state.as_str(), "queued" | "failed") {
            return Err(Error::Invalid("This message cannot return to drafts"));
        }
        let snapshot = snapshot.ok_or(Error::Invalid(
            "This older outbox entry has no recoverable draft snapshot",
        ))?;
        let mut draft: Draft = serde_json::from_str(&snapshot)?;
        let exists: bool = transaction.query_row("SELECT EXISTS(SELECT 1 FROM drafts WHERE id=?1 UNION ALL SELECT 1 FROM outbox WHERE draft_id=?1)", [new_id], |row|row.get(0))?;
        if new_id.is_empty() || exists {
            return Err(Error::Invalid("Choose a fresh draft ID"));
        }
        draft.id = new_id.to_owned();
        Self::persist_draft(&transaction, &draft)?;
        transaction.execute("UPDATE outbox SET state='cancelled',detail='Returned to drafts',restored_draft_id=?2 WHERE id=?1", params![id,new_id])?;
        transaction.commit()?;
        Ok(draft)
    }

    /// Call only while holding the process-wide application lock at startup.
    pub fn recover_interrupted_sends(&self) -> Result<()> {
        self.connection.execute("UPDATE outbox SET state='unknown',detail='Application stopped during submission. Check Sent before composing a replacement.' WHERE state='sending'", [])?;
        Ok(())
    }
}
