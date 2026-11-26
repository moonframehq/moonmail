use {
    crate::{Archive, Error, Result, SyncMessage},
    moonmail_core::MessageId,
    rusqlite::{OptionalExtension, params},
};

impl Archive {
    /// Link cached Gmail content in one durable transaction per inventory page.
    /// Return only the UIDs that still require a body download.
    pub fn link_cached_messages(
        &mut self,
        mailbox: i64,
        epoch: u32,
        page: Vec<SyncMessage>,
    ) -> Result<Vec<SyncMessage>> {
        if !page.iter().any(|message| message.gmail.is_some()) {
            return Ok(page);
        }
        let tx = self.connection.transaction()?;
        let current: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sync_scope c JOIN mailboxes m ON m.id=c.mailbox
             WHERE c.mailbox=?1 AND c.epoch=?2 AND c.complete AND c.generation=m.inventory_generation)",
            params![mailbox, epoch], |row| row.get(0),
        )?;
        if !current {
            return Err(Error::Invalid("Download inventory was superseded"));
        }
        let mut missing = Vec::new();
        for message in page {
            if let Some(gmail) = &message.gmail
                && link_occurrence(&tx, mailbox, epoch, &message, gmail)?
            {
                continue;
            }
            missing.push(message);
        }
        tx.commit()?;
        Ok(missing)
    }

    /// Replace a label intent with a fresh operation ID. One Gmail UID addresses
    /// the message across all labels. Prefer All Mail for a stable source UID.
    pub fn set_gmail_labels(
        &mut self,
        ids: &[MessageId],
        label: &str,
        value: bool,
    ) -> Result<usize> {
        if ids.is_empty() || ids.len() > 100 || !valid_gmail_label(label) {
            return Err(Error::Invalid(
                "Choose a valid Gmail label and up to 100 messages",
            ));
        }
        let tx = self.connection.transaction()?;
        for id in ids {
            queue_label(&tx, *id, label, value)?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    pub fn gmail_labels(&self, id: MessageId) -> Result<Option<Vec<String>>> {
        let labels: Option<String> = self.connection.query_row(
            "SELECT gmail_labels FROM messages WHERE id=?1",
            [id.0 as i64],
            |row| row.get(0),
        )?;
        labels
            .map(|json| serde_json::from_str(&json).map_err(Error::from))
            .transpose()
    }
}

fn link_occurrence(
    tx: &rusqlite::Transaction<'_>,
    mailbox: i64,
    epoch: u32,
    message: &SyncMessage,
    gmail: &crate::GmailMetadata,
) -> Result<bool> {
    let id: Option<i64> = tx
        .query_row(
            "SELECT m.id FROM messages m JOIN mailboxes b ON b.account=m.account
             WHERE b.id=?1 AND b.epoch=?2 AND m.gm_msgid=?3",
            params![mailbox, epoch, gmail.msgid.to_be_bytes()],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else { return Ok(false) };
    tx.execute(
        "INSERT INTO occurrences(mailbox_id,message_id,epoch,uid,seen,flagged)
             VALUES(?1,?2,?3,?4,?5,?6)",
        params![
            mailbox,
            id,
            epoch,
            message.uid,
            message.seen,
            message.starred
        ],
    )?;
    tx.execute(
        "UPDATE messages SET gmail_labels=?2,gm_thrid=?3 WHERE id=?1",
        params![
            id,
            serde_json::to_string(&gmail.labels)?,
            gmail.thrid.to_be_bytes()
        ],
    )?;
    crate::keywords::replace_observed(tx, mailbox, id, &message.keywords)?;
    crate::rules::on_entry(tx, mailbox, MessageId(id as u64))?;
    Ok(true)
}

pub fn valid_gmail_label(label: &str) -> bool {
    !label.is_empty() && label.len() <= 1024 && !label.chars().any(char::is_control)
}

pub(crate) fn queue_label(
    tx: &rusqlite::Transaction<'_>,
    id: MessageId,
    label: &str,
    value: bool,
) -> Result<()> {
    let id = id.0 as i64;
    let moving:bool=tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM move_operations WHERE message_id=?1 AND state IN ('queued','running','unknown'))",
        [id],|row|row.get(0)
    )?;
    if moving {
        return Err(Error::Invalid(
            "Finish pending transfers before editing labels",
        ));
    }
    let (mailbox,epoch,uid):(i64,u32,u32)=tx.query_row(
        "SELECT o.mailbox_id,o.epoch,o.uid FROM occurrences o
         JOIN mailboxes b ON b.id=o.mailbox_id JOIN messages m ON m.id=o.message_id
         WHERE o.message_id=?1 AND m.gm_msgid IS NOT NULL AND o.uid IS NOT NULL AND b.active AND b.selectable
         ORDER BY (b.server_role='archive') IS TRUE DESC,b.sync_enabled DESC,b.id LIMIT 1",
        [id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))
    ).optional()?.ok_or(Error::Invalid("Select a synchronized Gmail message"))?;
    tx.execute(
        "DELETE FROM keyword_operations AS f WHERE gmail=1 AND keyword=?2 AND EXISTS(
             SELECT 1 FROM occurrences o WHERE o.message_id=?1 AND o.mailbox_id=f.mailbox_id AND o.uid=f.uid AND o.epoch=f.epoch)",
        params![id,label]
    )?;
    tx.execute(
        "INSERT INTO keyword_operations(mailbox_id,epoch,uid,keyword,value,gmail) VALUES(?1,?2,?3,?4,?5,1)",
        params![mailbox,epoch,uid,label,value]
    )?;
    Ok(())
}

/// Before v21 each folder had its own message row. Rebind its live occurrence
/// when a prior scan has identified the same message. Keep historical message
/// IDs and bytes for drafts and operation history. A pending transfer retains
/// its original source identity until a later scan.
pub(crate) fn reconcile(tx: &rusqlite::Transaction<'_>, mailbox: i64, epoch: u32) -> Result<()> {
    tx.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS gmail_relinks(old_id INTEGER PRIMARY KEY,new_id INTEGER NOT NULL);
         DELETE FROM gmail_relinks;"
    )?;
    let changes = tx.execute(
        "INSERT INTO gmail_relinks
         SELECT old.id,known.id FROM occurrences o
         JOIN sync_inventory s ON s.uid=o.uid
         JOIN messages old ON old.id=o.message_id
         JOIN messages known ON known.account=old.account AND known.gm_msgid=s.gm_msgid
         WHERE o.mailbox_id=?1 AND o.epoch=?2 AND old.gm_msgid IS NULL
           AND NOT EXISTS(SELECT 1 FROM move_operations op WHERE op.message_id=old.id
               AND op.state IN ('queued','running','unknown'))",
        params![mailbox, epoch],
    )?;
    if changes > 0 {
        // Update both sides of the composite foreign key within this transaction.
        tx.pragma_update(None, "defer_foreign_keys", true)?;
        for table in ["occurrence_keywords", "occurrences"] {
            tx.execute(&format!("UPDATE {table} AS o SET message_id=r.new_id FROM gmail_relinks r WHERE o.mailbox_id=?1 AND o.message_id=r.old_id"),[mailbox])?;
        }
    }
    tx.execute(
        "UPDATE messages AS m SET gm_msgid=s.gm_msgid,gm_thrid=s.gm_thrid,gmail_labels=s.labels
         FROM occurrences o JOIN sync_inventory s ON o.uid=s.uid
         WHERE o.mailbox_id=?1 AND o.epoch=?2 AND o.message_id=m.id AND s.gm_msgid IS NOT NULL
           AND (m.gm_msgid=s.gm_msgid OR (m.gm_msgid IS NULL AND NOT EXISTS(
             SELECT 1 FROM messages other WHERE other.account=m.account AND other.gm_msgid=s.gm_msgid)))
           AND (m.gm_msgid IS NULL OR m.gm_thrid IS NOT s.gm_thrid OR m.gmail_labels IS NOT s.labels)",
        params![mailbox,epoch]
    )?;
    Ok(())
}
