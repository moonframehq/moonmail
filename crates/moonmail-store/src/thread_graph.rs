use {
    crate::{Error, Result},
    rusqlite::{Connection, OptionalExtension, params},
};

/// Account-scoped connected reference components. Missing RFC IDs are explicit
/// nodes. Message nodes stay distinct: conflicting duplicate IDs lose their own
/// ID links, so they cannot silently fuse unrelated conversations.
pub(crate) const KEY: &str = "COALESCE((SELECT 'graph:' || n.component FROM thread_members t JOIN thread_nodes n ON n.id=t.node_id WHERE t.message_id=m.id),'legacy:' || m.thread_key)";

pub(crate) fn relation(parameter: &str) -> String {
    format!("(m.account,{KEY})=(SELECT m.account,{KEY} FROM messages m WHERE m.id={parameter})")
}

fn valid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 998
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>'))
}

fn node(db: &Connection, account: &str, kind: &str, value: &str) -> Result<i64> {
    db.execute(
        "INSERT OR IGNORE INTO thread_nodes(account,kind,value) VALUES (?1,?2,?3)",
        params![account, kind, value],
    )?;
    let id = db.query_row(
        "SELECT id FROM thread_nodes WHERE account=?1 AND kind=?2 AND value=?3",
        params![account, kind, value],
        |row| row.get(0),
    )?;
    db.execute(
        "UPDATE thread_nodes SET component=id WHERE id=?1 AND component=0",
        [id],
    )?;
    db.execute(
        "INSERT OR IGNORE INTO thread_dirty SELECT component FROM thread_nodes WHERE id=?1",
        [id],
    )?;
    Ok(id)
}

fn edge(db: &Connection, source: i64, a: i64, b: i64, kind: &str) -> Result<()> {
    if a != b {
        db.execute(
            "INSERT OR IGNORE INTO thread_edges VALUES (?1,?2,?3,?4)",
            params![source, a.min(b), a.max(b), kind],
        )?;
    }
    Ok(())
}

/// Caller owns the metadata transaction. Header metadata, links, component
/// merges/splits and the pending marker become visible together.
pub(crate) fn update(db: &Connection, id: i64) -> Result<()> {
    let (account,rfc,blob,references,reply,legacy):(String,Option<String>,String,Option<String>,Option<String>,String)=db.query_row(
        "SELECT account,rfc_id,blob,references_json,in_reply_to,thread_key FROM messages WHERE id=?1",[id],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?,row.get(3)?,row.get(4)?,row.get(5)?)))?;
    db.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS thread_dirty(component INTEGER PRIMARY KEY);
        CREATE TEMP TABLE IF NOT EXISTS thread_remainder(id INTEGER PRIMARY KEY);
        CREATE TEMP TABLE IF NOT EXISTS thread_reached(id INTEGER PRIMARY KEY);
        DELETE FROM thread_dirty; DELETE FROM thread_remainder; DELETE FROM thread_reached;",
    )?;
    db.execute("INSERT OR IGNORE INTO thread_dirty SELECT n.component FROM thread_nodes n JOIN thread_edges e ON n.id=e.a OR n.id=e.b WHERE e.source=?1",[id])?;
    let mut split = db.execute("DELETE FROM thread_edges WHERE source=?1", [id])? > 0;
    let own = node(db, &account, "message", &id.to_string())?;
    let invalid_id = rfc.as_deref().is_some_and(|value| !valid(value));
    let rfc = rfc.as_deref().filter(|value| valid(value));
    let mut chain: Vec<String> = match references {
        Some(json) => serde_json::from_str(&json)?,
        // A pre-header-storage archive retains its former grouping until raw
        // headers are recovered explicitly. Migration never opens MIME bodies.
        None => {
            if rfc != Some(legacy.as_str()) && valid(&legacy) {
                vec![legacy]
            } else {
                Vec::new()
            }
        },
    };
    if chain.is_empty()
        && let Some(reply) = reply
    {
        chain.push(reply);
    }
    let mut limited = invalid_id || chain.iter().any(|value| !valid(value));
    chain.retain(|value| valid(value) && Some(value.as_str()) != rfc);
    chain.dedup();
    // Original headers remain complete. Bound graph fan-out per message while
    // retaining the oldest ancestor and nearest 255 references.
    if chain.len() > 256 {
        limited = true;
        chain.drain(1..chain.len() - 255);
    }
    db.execute("INSERT INTO thread_members VALUES (?1,?2,?3) ON CONFLICT(message_id) DO UPDATE SET limited=excluded.limited",params![id,own,limited])?;
    let mut previous = None;
    for reference in chain {
        let next = node(db, &account, "rfc", &reference)?;
        if let Some(previous) = previous {
            edge(db, id, previous, next, "reference")?;
        }
        previous = Some(next);
    }
    if let Some(previous) = previous {
        edge(db, id, previous, own, "reference")?;
    }
    if let Some(rfc) = rfc {
        let duplicate: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE account=?1 AND rfc_id=?2 AND blob<>?3)",
            params![account, rfc, blob],
            |row| row.get(0),
        )?;
        if duplicate {
            db.execute("INSERT OR IGNORE INTO thread_dirty SELECT n.component FROM thread_nodes n JOIN thread_edges e ON n.id=e.a OR n.id=e.b JOIN messages m ON m.id=e.source WHERE m.account=?1 AND m.rfc_id=?2 AND e.kind='own'",params![account,rfc])?;
            split|=db.execute("DELETE FROM thread_edges WHERE kind='own' AND source IN (SELECT id FROM messages WHERE account=?1 AND rfc_id=?2)",params![account,rfc])?>0;
        } else {
            let rfc_node = node(db, &account, "rfc", rfc)?;
            edge(db, id, own, rfc_node, "own")?;
        }
    }
    if split {
        split_components(db)?;
    } else {
        db.execute("UPDATE thread_nodes SET component=(SELECT MIN(component) FROM thread_dirty) WHERE component IN (SELECT component FROM thread_dirty) AND component<>(SELECT MIN(component) FROM thread_dirty)",[])?;
    }
    db.execute("DELETE FROM thread_pending WHERE message_id=?1", [id])?;
    Ok(())
}

fn split_components(db: &Connection) -> Result<()> {
    db.execute("INSERT INTO thread_remainder SELECT id FROM thread_nodes WHERE component IN (SELECT component FROM thread_dirty)",[])?;
    loop {
        let root: Option<i64> = db
            .query_row(
                "SELECT id FROM thread_remainder ORDER BY id LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        let Some(root) = root else {
            break;
        };
        db.execute("WITH RECURSIVE reached(id) AS (SELECT ?1 UNION SELECT e.b FROM thread_edges e JOIN reached r ON e.a=r.id UNION SELECT e.a FROM thread_edges e JOIN reached r ON e.b=r.id) INSERT INTO thread_reached SELECT id FROM reached",[root])?;
        db.execute("UPDATE thread_nodes SET component=?1 WHERE id IN (SELECT id FROM thread_reached) AND component<>?1",[root])?;
        db.execute_batch("DELETE FROM thread_remainder WHERE id IN (SELECT id FROM thread_reached); DELETE FROM thread_reached;")?;
    }
    Ok(())
}

/// Resumable metadata-only migration. Each bounded page commits graph state and
/// its work markers atomically. Opening does not expose partially migrated views.
pub(crate) fn backfill(db: &mut Connection) -> Result<()> {
    loop {
        let transaction = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ids = transaction
            .prepare("SELECT message_id FROM thread_pending ORDER BY message_id LIMIT 100")?
            .query_map([], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if ids.is_empty() {
            return Ok(());
        }
        for id in ids {
            update(&transaction, id)?;
        }
        transaction.commit()?;
    }
}

impl crate::Archive {
    /// Bounded metadata diagnostics for the selected message, without MIME reads.
    pub fn thread_warnings(&self, id: moonmail_core::MessageId) -> Result<Vec<&'static str>> {
        let (limited, conflict): (bool, bool) = self.connection.query_row(
            "SELECT t.limited,EXISTS(SELECT 1 FROM messages other WHERE other.account=m.account AND other.rfc_id=m.rfc_id AND other.blob<>m.blob)
             FROM thread_members t JOIN messages m ON m.id=t.message_id WHERE m.id=?1",
            [id.0 as i64], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?.ok_or(Error::NotFound)?;
        let mut warnings = Vec::new();
        if limited {
            warnings.push("Thread links are limited: malformed or excessive references were omitted. Original headers are preserved.");
        }
        if conflict {
            warnings.push("Conflicting Message-ID: automatic ID links are disabled; explicit references still apply.");
        }
        Ok(warnings)
    }

    pub fn limited_thread_count(&self) -> Result<u64> {
        let count: i64 = self.connection.query_row(
            "SELECT COUNT(*) FROM thread_members WHERE limited=1",
            [],
            |row| row.get(0),
        )?;
        u64::try_from(count).map_err(|_| Error::Invalid("Invalid thread count"))
    }
}
