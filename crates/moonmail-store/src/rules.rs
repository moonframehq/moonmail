use {
    crate::{Archive, Error, Result},
    moonmail_core::{MailPredicate, MessageId, canonical_keyword},
    rusqlite::{Connection, OptionalExtension, params, types::Value},
    serde::{Deserialize, Serialize},
};

/// One metadata predicate and one keyword action. Folder entry is physical
/// occurrence creation, not every sync, reindex, label edit or repeated local copy.
/// Automatic rules run in ID order in the occurrence's transaction; later rules
/// see earlier actions. No action creates folder entries, so cascades terminate.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeywordRule {
    pub id: i64,
    pub revision: i64,
    pub mailbox: i64,
    pub name: String,
    pub source: String,
    pub predicate: MailPredicate,
    pub keyword: String,
    pub value: bool,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct KeywordRuleRun {
    pub id: i64,
    pub rule_id: i64,
    pub revision: i64,
    pub name: String,
    pub mailbox: i64,
    pub message: MessageId,
    pub keyword: String,
    pub value: bool,
    pub mode: String,
    pub state: String,
    pub detail: String,
}

fn decode(row: &rusqlite::Row<'_>) -> rusqlite::Result<(KeywordRule, String)> {
    Ok((
        KeywordRule {
            id: row.get(0)?,
            revision: row.get(1)?,
            mailbox: row.get(2)?,
            name: row.get(3)?,
            source: row.get(4)?,
            predicate: MailPredicate::Unread,
            keyword: row.get(6)?,
            value: row.get(7)?,
            enabled: row.get(8)?,
        },
        row.get(5)?,
    ))
}
fn list(db: &Connection, mailbox: Option<i64>) -> Result<Vec<KeywordRule>> {
    let mut query=db.prepare("SELECT id,revision,mailbox_id,name,source,predicate,keyword,value,enabled FROM keyword_rules WHERE (?1 IS NULL OR mailbox_id=?1) ORDER BY id LIMIT 100")?;
    query
        .query_map([mailbox], decode)?
        .map(|row| {
            let (mut rule, predicate) = row?;
            rule.predicate = serde_json::from_str(&predicate)?;
            Ok(rule)
        })
        .collect()
}

fn compiled(rule: &KeywordRule) -> Result<crate::predicates::CompiledPredicate> {
    let compiled = crate::predicates::compile(&rule.predicate)?;
    if !compiled.terms.is_empty() {
        return Err(Error::Invalid(
            "Entry rules accept metadata conditions only: account, in, is, keyword, tag, sender-address, recipient-address, after and before",
        ));
    }
    Ok(compiled)
}

fn run(db: &Connection, rule: &KeywordRule, id: MessageId, mode: &str) -> Result<bool> {
    let compiled = compiled(rule)?;
    let mut args = vec![Value::from(id.0 as i64), Value::from(rule.mailbox)];
    args.extend(compiled.arguments);
    let matches:bool=db.query_row(&format!("SELECT EXISTS(SELECT 1 FROM messages m WHERE m.id=? AND EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id AND o.mailbox_id=?) AND ({}))",compiled.sql),rusqlite::params_from_iter(args),|r|r.get(0))?;
    if !matches {
        return Ok(false);
    }
    let (state, detail) = match crate::keywords::set_one(db, id, &rule.keyword, rule.value) {
        Ok(()) => (
            "applied",
            "Keyword action committed; remote intent uploads on Sync",
        ),
        Err(Error::Invalid(detail)) => ("blocked", detail),
        Err(error) => return Err(error),
    };
    db.execute("INSERT INTO keyword_rule_runs(rule_id,revision,name,mailbox_id,message_id,epoch,uid,keyword,value,mode,state,detail)
        SELECT ?1,?2,?3,mailbox_id,message_id,epoch,uid,?6,?7,?8,?9,?10 FROM occurrences WHERE mailbox_id=?4 AND message_id=?5",
        params![rule.id,rule.revision,rule.name,rule.mailbox,id.0 as i64,rule.keyword,rule.value,mode,state,detail])?;
    Ok(state == "applied")
}

pub(crate) fn on_entry(db: &Connection, mailbox: i64, id: MessageId) -> Result<()> {
    for rule in list(db, Some(mailbox))?.into_iter().filter(|r| r.enabled) {
        run(db, &rule, id, "entry")?;
    }
    Ok(())
}

impl Archive {
    /// Create with id/revision zero; replace with exact current id/revision.
    /// Saving/enabling has no retrospective effects on existing folder members.
    pub fn save_keyword_rule(&mut self, rule: &KeywordRule) -> Result<i64> {
        compiled(rule)?;
        let keyword =
            canonical_keyword(&rule.keyword).ok_or(Error::Invalid("Invalid IMAP keyword"))?;
        if rule.name.trim().is_empty()
            || rule.name.len() > 100
            || rule.name.chars().any(char::is_control)
            || rule.source.len() > 4096
        {
            return Err(Error::Invalid("Invalid rule name or condition"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let active: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM mailboxes WHERE id=?1 AND active=1 AND selectable=1)",
            [rule.mailbox],
            |r| r.get(0),
        )?;
        if !active {
            return Err(Error::Invalid(
                "Choose an active selectable rule source folder",
            ));
        }
        let predicate = serde_json::to_string(&rule.predicate)?;
        let id = if rule.id == 0 && rule.revision == 0 {
            let count: i64 =
                tx.query_row("SELECT COUNT(*) FROM keyword_rules", [], |r| r.get(0))?;
            if count >= 100 {
                return Err(Error::Invalid("At most 100 entry rules"));
            }
            tx.execute("INSERT INTO keyword_rules(mailbox_id,name,source,predicate,keyword,value,enabled) VALUES (?1,?2,?3,?4,?5,?6,?7)",params![rule.mailbox,rule.name,rule.source,predicate,keyword,rule.value,rule.enabled])?;
            tx.last_insert_rowid()
        } else {
            if tx.execute("UPDATE keyword_rules SET revision=revision+1,mailbox_id=?3,name=?4,source=?5,predicate=?6,keyword=?7,value=?8,enabled=?9 WHERE id=?1 AND revision=?2",params![rule.id,rule.revision,rule.mailbox,rule.name,rule.source,predicate,keyword,rule.value,rule.enabled])?!=1 {
                return Err(Error::Invalid("Rule changed; reopen its editor"));
            }
            rule.id
        };
        tx.commit()?;
        Ok(id)
    }

    pub fn keyword_rules(&self) -> Result<Vec<KeywordRule>> {
        list(&self.connection, None)
    }

    pub fn delete_keyword_rule(&self, id: i64, revision: i64) -> Result<()> {
        if self.connection.execute(
            "DELETE FROM keyword_rules WHERE id=?1 AND revision=?2",
            params![id, revision],
        )? != 1
        {
            return Err(Error::Invalid("Rule changed; reopen its editor"));
        }
        Ok(())
    }

    /// Explicit selected messages only, even for a disabled rule. Recheck its
    /// predicate and source; one stale selection aborts all actions/log entries.
    pub fn apply_keyword_rule(
        &mut self,
        id: i64,
        revision: i64,
        ids: &[MessageId],
    ) -> Result<usize> {
        if ids.is_empty() || ids.len() > 100 {
            return Err(Error::Invalid("Choose between one and 100 messages"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let rule = list(&tx, None)?
            .into_iter()
            .find(|r| r.id == id && r.revision == revision)
            .ok_or(Error::Invalid("Rule changed; reopen its editor"))?;
        let ids: std::collections::BTreeSet<_> = ids.iter().copied().collect();
        let mut count = 0;
        for message in ids {
            let present:Option<i64>=tx.query_row("SELECT 1 FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id WHERE o.message_id=?1 AND o.mailbox_id=?2 AND b.active=1 AND b.selectable=1",params![message.0 as i64,rule.mailbox],|r|r.get(0)).optional()?;
            if present.is_none() {
                return Err(Error::Invalid(
                    "A selected message is no longer in the rule source folder",
                ));
            }
            count += usize::from(run(&tx, &rule, message, "manual")?);
        }
        tx.commit()?;
        Ok(count)
    }

    pub fn keyword_rule_runs(&self, before: Option<i64>) -> Result<Vec<KeywordRuleRun>> {
        let mut query=self.connection.prepare("SELECT id,rule_id,revision,name,mailbox_id,message_id,keyword,value,mode,state,detail FROM keyword_rule_runs WHERE (?1 IS NULL OR id<?1) ORDER BY id DESC LIMIT 100")?;
        Ok(query
            .query_map([before], |r| {
                Ok(KeywordRuleRun {
                    id: r.get(0)?,
                    rule_id: r.get(1)?,
                    revision: r.get(2)?,
                    name: r.get(3)?,
                    mailbox: r.get(4)?,
                    message: MessageId(r.get::<_, i64>(5)? as u64),
                    keyword: r.get(6)?,
                    value: r.get(7)?,
                    mode: r.get(8)?,
                    state: r.get(9)?,
                    detail: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }
}
