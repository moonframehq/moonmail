//! Read-only mirrors of msgvault archives. msgvault stays authoritative for
//! which folders hold a message and whether it is read; Moonmail stores each
//! message's original bytes once and follows later changes.
use {
    crate::{Archive, Error, Result},
    moonmail_core::{MailboxRole, MessageId},
    rusqlite::params,
    std::collections::HashMap,
};

pub struct MsgvaultAccount {
    pub account: String,
    pub database: String,
    pub source: i64,
    /// Every msgvault message from `backfill` up to this ID is mirrored or
    /// deliberately skipped; newer messages arrive above it.
    pub cursor: i64,
    /// -1 before the first import, 0 once older mail is complete, otherwise
    /// the lowest ID mirrored while importing newest first.
    pub backfill: i64,
}

/// What Moonmail last observed in msgvault for one mirrored message.
pub struct MsgvaultState {
    pub message: MessageId,
    pub seen: bool,
    /// Sorted label names joined by U+001F.
    pub labels: String,
}

/// A newly observed msgvault state. `mailboxes` is `None` once the message
/// was deleted from msgvault's source.
pub struct MsgvaultChange<'a> {
    pub vault: i64,
    pub seen: bool,
    pub labels: &'a str,
    pub mailboxes: Option<&'a [i64]>,
}

pub(crate) fn check_editable(db: &rusqlite::Connection, account: &str) -> Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM msgvault_accounts WHERE account=?1)",
        [account],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Invalid(
            "msgvault accounts are read-only; move and organize mail in msgvault's source",
        ));
    }
    Ok(())
}

impl Archive {
    /// Returns whether the account is new. A source already mirrored keeps its
    /// cursor, and a source the user removed stays removed.
    pub fn register_msgvault(
        &mut self,
        account: &str,
        database: &str,
        source: i64,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            "INSERT INTO msgvault_accounts(account,database,source,backfill)
             SELECT ?1,?2,?3,-1 WHERE NOT EXISTS(
                 SELECT 1 FROM msgvault_dismissed WHERE database=?2 AND source=?3)
             ON CONFLICT DO NOTHING",
            params![account, database, source],
        )? > 0)
    }

    /// Start a newest-first import at `newest`, the highest msgvault ID now.
    pub fn begin_msgvault(&self, account: &str, newest: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE msgvault_accounts SET cursor=?2,backfill=?2+1 WHERE account=?1 AND backfill<0",
            params![account, newest],
        )?;
        Ok(())
    }

    pub fn finish_msgvault_backfill(&self, account: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE msgvault_accounts SET backfill=0 WHERE account=?1 AND backfill>0",
            [account],
        )?;
        Ok(())
    }

    /// Hide a mirrored account and stop detecting its source. Originals stay
    /// in the archive; `forget_removed_msgvault` lets detection add it again.
    pub fn remove_msgvault(&mut self, account: &str) -> Result<()> {
        let tx = self.connection.transaction()?;
        if tx.execute(
            "INSERT OR IGNORE INTO msgvault_dismissed
             SELECT database,source FROM msgvault_accounts WHERE account=?1",
            [account],
        )? == 0
            && !tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM msgvault_accounts WHERE account=?1)",
                [account],
                |r| r.get::<_, bool>(0),
            )?
        {
            return Err(Error::Invalid("No msgvault account has this name"));
        }
        tx.execute(
            "DELETE FROM occurrences WHERE mailbox_id IN (SELECT id FROM mailboxes WHERE account=?1)",
            [account],
        )?;
        tx.execute("UPDATE mailboxes SET active=0 WHERE account=?1", [account])?;
        tx.execute("DELETE FROM msgvault_messages WHERE account=?1", [account])?;
        tx.execute("DELETE FROM msgvault_accounts WHERE account=?1", [account])?;
        tx.commit()?;
        Ok(())
    }

    pub fn forget_removed_msgvault(&self) -> Result<()> {
        self.connection
            .execute("DELETE FROM msgvault_dismissed", [])?;
        Ok(())
    }

    pub fn msgvault_accounts(&self) -> Result<Vec<MsgvaultAccount>> {
        let mut q = self.connection.prepare(
            "SELECT account,database,source,cursor,backfill FROM msgvault_accounts ORDER BY account",
        )?;
        Ok(q.query_map([], |r| {
            Ok(MsgvaultAccount {
                account: r.get(0)?,
                database: r.get(1)?,
                source: r.get(2)?,
                cursor: r.get(3)?,
                backfill: r.get(4)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }

    pub fn msgvault_mailbox(
        &self,
        account: &str,
        name: &str,
        role: Option<MailboxRole>,
    ) -> Result<i64> {
        let id = self.mailbox(account, name)?;
        self.mailbox_server_role(id, role)?;
        Ok(id)
    }

    /// Publish a message's msgvault folders and advance the cursor atomically.
    /// Returns false, and hides the message, once the account was removed.
    /// The message itself was ingested before, so a retry finds it by origin.
    pub fn record_msgvault(
        &mut self,
        account: &str,
        message: MessageId,
        change: &MsgvaultChange<'_>,
    ) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let id = message.0 as i64;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM msgvault_accounts WHERE account=?1)",
            [account],
            |r| r.get(0),
        )?;
        if !exists {
            // Removed while this message was being mirrored: undo its ingest.
            tx.execute(
                "DELETE FROM occurrences WHERE message_id=?1
                 AND mailbox_id IN (SELECT id FROM mailboxes WHERE account=?2)",
                params![id, account],
            )?;
            tx.execute("UPDATE mailboxes SET active=0 WHERE account=?1", [account])?;
            tx.commit()?;
            return Ok(false);
        }
        for mailbox in change.mailboxes.unwrap_or_default() {
            add_occurrence(&tx, *mailbox, id, change.seen, starred(change.labels))?;
        }
        tx.execute(
            "INSERT INTO msgvault_messages VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(message) DO UPDATE SET seen=excluded.seen,labels=excluded.labels",
            params![id, account, change.vault, change.seen, change.labels],
        )?;
        advance(&tx, account, change.vault)?;
        tx.commit()?;
        Ok(true)
    }

    /// Advance past a msgvault message that has no mail to mirror.
    pub fn skip_msgvault(&self, account: &str, vault: i64) -> Result<()> {
        advance(&self.connection, account, vault)
    }

    pub fn msgvault_states(&self, account: &str) -> Result<HashMap<i64, MsgvaultState>> {
        let mut q = self
            .connection
            .prepare("SELECT vault,message,seen,labels FROM msgvault_messages WHERE account=?1")?;
        Ok(q.query_map([account], |r| {
            Ok((
                r.get(0)?,
                MsgvaultState {
                    message: MessageId(r.get::<_, i64>(1)? as u64),
                    seen: r.get(2)?,
                    labels: r.get(3)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?)
    }

    /// Apply msgvault's changes since the last observation. A read state set
    /// in Moonmail stays until msgvault's own read state changes.
    pub fn update_msgvault(
        &mut self,
        account: &str,
        changes: &[(&MsgvaultState, MsgvaultChange<'_>)],
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        for (old, change) in changes {
            let id = old.message.0 as i64;
            let Some(mailboxes) = change.mailboxes else {
                tx.execute(
                    "DELETE FROM occurrences WHERE message_id=?1
                     AND mailbox_id IN (SELECT id FROM mailboxes WHERE account=?2)",
                    params![id, account],
                )?;
                tx.execute("DELETE FROM msgvault_messages WHERE message=?1", [id])?;
                continue;
            };
            if old.labels != change.labels {
                tx.execute(
                    "DELETE FROM occurrences WHERE message_id=?1
                     AND mailbox_id IN (SELECT id FROM mailboxes WHERE account=?2)
                     AND mailbox_id NOT IN (SELECT value FROM json_each(?3))",
                    params![id, account, serde_json::to_string(mailboxes)?],
                )?;
                for mailbox in mailboxes {
                    add_occurrence(&tx, *mailbox, id, change.seen, starred(change.labels))?;
                }
                if starred(&old.labels) != starred(change.labels) {
                    tx.execute(
                        "UPDATE occurrences SET flagged=?2 WHERE message_id=?1",
                        params![id, starred(change.labels)],
                    )?;
                }
            }
            if old.seen != change.seen {
                tx.execute(
                    "UPDATE occurrences SET seen=?2 WHERE message_id=?1",
                    params![id, change.seen],
                )?;
            }
            tx.execute(
                "UPDATE msgvault_messages SET seen=?2,labels=?3 WHERE message=?1",
                params![id, change.seen, change.labels],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}

/// Gmail keeps stars as the STARRED label.
fn starred(labels: &str) -> bool {
    labels.split('\u{1f}').any(|label| label == "STARRED")
}

fn add_occurrence(
    tx: &rusqlite::Connection,
    mailbox: i64,
    id: i64,
    seen: bool,
    starred: bool,
) -> Result<()> {
    if tx.execute(
        "INSERT INTO occurrences(mailbox_id,message_id,epoch,seen,flagged)
         VALUES (?1,?2,0,?3,MAX(?4,COALESCE((SELECT MAX(flagged) FROM occurrences WHERE message_id=?2),0)))
         ON CONFLICT(mailbox_id,message_id) DO NOTHING",
        params![mailbox, id, seen, starred],
    )? > 0
    {
        crate::rules::on_entry(tx, mailbox, MessageId(id as u64))?;
    }
    Ok(())
}

/// New mail raises the cursor; the newest-first import lowers the backfill.
fn advance(db: &rusqlite::Connection, account: &str, vault: i64) -> Result<()> {
    db.execute(
        "UPDATE msgvault_accounts SET cursor=MAX(cursor,?2),
         backfill=CASE WHEN backfill>?2 THEN ?2 ELSE backfill END WHERE account=?1",
        params![account, vault],
    )?;
    Ok(())
}
