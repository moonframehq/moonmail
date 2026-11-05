use {
    crate::{Archive, Error, Result},
    moonmail_core::{MessageId, canonical_keyword},
    rusqlite::{Connection, params},
    serde::Serialize,
    std::collections::BTreeSet,
};

#[derive(Debug, Serialize)]
pub struct KeywordOperation {
    pub id: i64,
    pub mailbox: String,
    pub epoch: u32,
    pub uid: u32,
    pub keyword: String,
    pub value: bool,
    pub state: String,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct KeywordLabel {
    pub keyword: String,
    pub label: String,
}

pub(crate) fn normalize(values: &[String]) -> Result<Vec<String>> {
    if values.len() > 128 {
        return Err(Error::Invalid("At most 128 keywords per occurrence"));
    }
    values
        .iter()
        .map(|s| canonical_keyword(s).ok_or(Error::Invalid("Invalid IMAP keyword atom")))
        .collect::<Result<BTreeSet<_>>>()
        .map(|set| set.into_iter().collect())
}

pub(crate) fn replace_observed(
    db: &Connection,
    mailbox: i64,
    message: i64,
    keywords: &[String],
) -> Result<()> {
    db.execute(
        "DELETE FROM occurrence_keywords WHERE mailbox_id=?1 AND message_id=?2",
        params![mailbox, message],
    )?;
    for keyword in keywords {
        db.execute(
            "INSERT INTO occurrence_keywords VALUES (?1,?2,?3)",
            params![mailbox, message, keyword],
        )?;
    }
    // Overlay both queued and blocked local intent; a failed server operation
    // must never silently erase the user's visible desired state.
    db.execute("DELETE FROM occurrence_keywords AS k WHERE k.mailbox_id=?1 AND k.message_id=?2 AND EXISTS(
        SELECT 1 FROM keyword_operations f JOIN occurrences o ON o.mailbox_id=f.mailbox_id AND o.epoch=f.epoch AND o.uid=f.uid
        WHERE o.mailbox_id=k.mailbox_id AND o.message_id=k.message_id AND f.keyword=k.keyword AND f.value=0)",params![mailbox,message])?;
    db.execute("INSERT OR IGNORE INTO occurrence_keywords SELECT o.mailbox_id,o.message_id,f.keyword
        FROM occurrences o JOIN keyword_operations f ON f.mailbox_id=o.mailbox_id AND f.epoch=o.epoch AND f.uid=o.uid
        WHERE o.mailbox_id=?1 AND o.message_id=?2 AND f.value=1",params![mailbox,message])?;
    Ok(())
}

impl Archive {
    pub fn unresolved_keyword_count(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM keyword_operations", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }

    pub fn keyword_source_exists(&self, id: i64) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM keyword_operations f JOIN occurrences o ON o.mailbox_id=f.mailbox_id AND o.epoch=f.epoch AND o.uid=f.uid JOIN mailboxes b ON b.id=o.mailbox_id WHERE f.id=?1 AND b.active=1 AND b.selectable=1)",[id],|r|r.get(0))?)
    }

    pub fn has_keyword_intent(&self, mailbox: i64, epoch: u32, uid: u32) -> Result<bool> {
        Ok(self.connection.query_row("SELECT EXISTS(SELECT 1 FROM keyword_operations WHERE mailbox_id=?1 AND epoch=?2 AND uid=?3)",params![mailbox,epoch,uid],|r|r.get(0))?)
    }

    /// Atomic explicit selection, all live occurrences; pending transfers exclude
    /// edits. Each edit gets a fresh durable ID, including A/B/A replacements.
    pub fn set_keywords(&mut self, ids: &[MessageId], keyword: &str, value: bool) -> Result<usize> {
        let keyword =
            canonical_keyword(keyword).ok_or(Error::Invalid("Invalid IMAP keyword atom"))?;
        if ids.is_empty() || ids.len() > 100 {
            return Err(Error::Invalid("Choose between one and 100 messages"));
        }
        let ids: BTreeSet<_> = ids.iter().copied().collect();
        let tx = self.connection.transaction()?;
        for id in &ids {
            set_one(&tx, *id, &keyword, value)?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    pub fn message_keywords(&self, id: MessageId) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT DISTINCT keyword FROM occurrence_keywords WHERE message_id=?1 ORDER BY keyword",
        )?;
        Ok(statement
            .query_map([id.0 as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Empty label removes the mapping; wire keyword membership is unchanged.
    pub fn set_keyword_label(&self, account: &str, keyword: &str, label: &str) -> Result<()> {
        let keyword =
            canonical_keyword(keyword).ok_or(Error::Invalid("Invalid IMAP keyword atom"))?;
        if account.is_empty()
            || label.len() > 200
            || label.chars().any(char::is_control)
            || label.trim() != label
        {
            return Err(Error::Invalid("Invalid account or keyword label"));
        }
        if label.is_empty() {
            self.connection.execute(
                "DELETE FROM keyword_labels WHERE account=?1 AND keyword=?2",
                params![account, keyword],
            )?;
        } else {
            self.connection.execute("INSERT INTO keyword_labels VALUES (?1,?2,?3) ON CONFLICT(account,keyword) DO UPDATE SET label=excluded.label",params![account,keyword,label])?;
        }
        Ok(())
    }

    pub fn keyword_labels(&self, account: &str) -> Result<Vec<KeywordLabel>> {
        let mut statement = self.connection.prepare(
            "SELECT keyword,label FROM keyword_labels WHERE account=?1 ORDER BY label LIMIT 500",
        )?;
        Ok(statement
            .query_map([account], |r| {
                Ok(KeywordLabel {
                    keyword: r.get(0)?,
                    label: r.get(1)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Keyset-page unresolved operations, including blocked diagnostics.
    pub fn keyword_operations(&self, account: &str, after: i64) -> Result<Vec<KeywordOperation>> {
        let mut statement=self.connection.prepare("SELECT f.id,b.name,f.epoch,f.uid,f.keyword,f.value,f.state,f.detail FROM keyword_operations f JOIN mailboxes b ON b.id=f.mailbox_id WHERE b.account=?1 AND f.id>?2 ORDER BY f.id LIMIT 100")?;
        Ok(statement
            .query_map(params![account, after], |r| {
                Ok(KeywordOperation {
                    id: r.get(0)?,
                    mailbox: r.get(1)?,
                    epoch: r.get(2)?,
                    uid: r.get(3)?,
                    keyword: r.get(4)?,
                    value: r.get(5)?,
                    state: r.get(6)?,
                    detail: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn acknowledge_keyword(&self, id: i64) -> Result<()> {
        self.connection
            .execute("DELETE FROM keyword_operations WHERE id=?1", [id])?;
        Ok(())
    }

    pub fn block_keyword(&self, id: i64, detail: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE keyword_operations SET state='blocked',detail=?2 WHERE id=?1",
            params![id, detail],
        )?;
        Ok(())
    }

    /// Explicit retry is safe: keyword STORE adds/removes one atom idempotently.
    pub fn retry_keyword(&self, id: i64) -> Result<()> {
        if self.connection.execute("UPDATE keyword_operations SET state='queued',detail='' WHERE id=?1 AND state='blocked'",[id])?!=1 {
            return Err(Error::Invalid("Keyword operation is not blocked"));
        }
        Ok(())
    }

    /// Discard intent; the next complete server inventory restores observed state.
    pub fn discard_keyword(&self, id: i64) -> Result<()> {
        if self.connection.execute(
            "DELETE FROM keyword_operations WHERE id=?1 AND state='blocked'",
            [id],
        )? != 1
        {
            return Err(Error::Invalid("Keyword operation is not blocked"));
        }
        Ok(())
    }
}

/// Shared by explicit bulk edits and atomic folder-entry rules. Caller owns the
/// transaction and supplies a validated, canonical keyword.
pub(crate) fn set_one(db: &Connection, id: MessageId, keyword: &str, value: bool) -> Result<()> {
    let live: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM occurrences WHERE message_id=?1)",
        [id.0 as i64],
        |r| r.get(0),
    )?;
    if !live {
        return Err(Error::Invalid("A selected message has no live occurrence"));
    }
    let moving: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM move_operations WHERE message_id=?1 AND state IN ('queued','running','unknown'))",[id.0 as i64],|r|r.get(0))?;
    if moving {
        return Err(Error::Invalid(
            "Finish or cancel pending transfers before editing keywords",
        ));
    }
    if value {
        let full: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM occurrence_keywords k WHERE message_id=?1 GROUP BY mailbox_id HAVING COUNT(*)>=128 AND SUM(keyword=?2)=0)",params![id.0 as i64,keyword],|r|r.get(0))?;
        if full {
            return Err(Error::Invalid("At most 128 keywords per occurrence"));
        }
    }
    db.execute("DELETE FROM keyword_operations WHERE keyword=?2 AND EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=?1 AND o.mailbox_id=keyword_operations.mailbox_id AND o.epoch=keyword_operations.epoch AND o.uid=keyword_operations.uid)",params![id.0 as i64,keyword])?;
    db.execute("INSERT INTO keyword_operations(mailbox_id,epoch,uid,keyword,value) SELECT mailbox_id,epoch,uid,?2,?3 FROM occurrences WHERE message_id=?1 AND uid IS NOT NULL",params![id.0 as i64,keyword,value])?;
    if value {
        db.execute("INSERT OR IGNORE INTO occurrence_keywords SELECT mailbox_id,message_id,?2 FROM occurrences WHERE message_id=?1",params![id.0 as i64,keyword])?;
    } else {
        db.execute(
            "DELETE FROM occurrence_keywords WHERE message_id=?1 AND keyword=?2",
            params![id.0 as i64, keyword],
        )?;
    }
    Ok(())
}
