use {
    crate::{Error, Result},
    moonmail_core::{IndexChange, MessageId, SearchIndex, SearchQuery, TextField, TextTerm},
    rusqlite::{Connection, params},
    std::{path::Path, time::Duration},
};

/// Owns one blocking connection; move the adapter to a dedicated worker.
///
/// Disk databases use WAL and synchronous=FULL. `apply` commits both the stored
/// searchable text and FTS changes in one transaction. This does not atomically
/// commit a separate authoritative metadata database or its synchronization cursor.
pub struct SqliteIndex {
    connection: Connection,
}

impl SqliteIndex {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(path)?;
        Self::initialize(connection)
    }

    /// For contract tests and explicitly labeled non-durable benchmarks.
    pub fn in_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }

    fn initialize(mut connection: Connection) -> Result<Self> {
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        let transaction = connection.transaction()?;
        let version = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => transaction.execute_batch(include_str!("schema.sql"))?,
            1 => {},
            version => return Err(Error::SchemaVersion(version)),
        }
        transaction.commit()?;
        Ok(Self { connection })
    }
}

impl SearchIndex for SqliteIndex {
    type Error = Error;

    fn visit_terms(
        &self,
        terms: &[TextTerm],
        mut visitor: impl FnMut(usize, MessageId) -> bool,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        {
            let mut query = transaction.prepare(
                "SELECT rowid FROM documents_fts WHERE documents_fts MATCH ?1 ORDER BY rowid",
            )?;
            'terms: for (slot, term) in terms.iter().enumerate() {
                let phrase = expression(&[&term.value]);
                if phrase.is_empty() {
                    continue;
                }
                let field = match term.field {
                    TextField::Any => None,
                    TextField::Subject => Some("subject"),
                    TextField::Body => Some("body"),
                    TextField::From => Some("from_addr"),
                    TextField::To => Some("to_addr"),
                    TextField::Cc => Some("cc_addr"),
                };
                let expression =
                    field.map_or_else(|| phrase.clone(), |field| format!("{field} : {phrase}"));
                let mut rows = query.query([expression])?;
                while let Some(row) = rows.next()? {
                    let id: i64 = row.get(0)?;
                    let id = u64::try_from(id)
                        .map(MessageId)
                        .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, id))?;
                    if !visitor(slot, id) {
                        break 'terms;
                    }
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn apply(&mut self, changes: &[IndexChange<'_>]) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        let transaction = self.connection.transaction()?;
        {
            let mut upsert = transaction.prepare_cached(
                "INSERT INTO search_documents(id, subject, body, from_addr, to_addr, cc_addr)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(id) DO UPDATE SET subject=excluded.subject, body=excluded.body,
                 from_addr=excluded.from_addr, to_addr=excluded.to_addr, cc_addr=excluded.cc_addr",
            )?;
            let mut delete =
                transaction.prepare_cached("DELETE FROM search_documents WHERE id=?1")?;
            for change in changes {
                match change {
                    IndexChange::Upsert(id, document) => {
                        upsert.execute(params![
                            sqlite_id(*id)?,
                            document.subject,
                            document.body,
                            document.from,
                            document.to,
                            document.cc,
                        ])?;
                    },
                    IndexChange::Delete(id) => {
                        delete.execute([sqlite_id(*id)?])?;
                    },
                }
            }
        }
        transaction.commit()?;
        Ok(())
    }

    fn search(&self, query: SearchQuery<'_>) -> Result<Vec<MessageId>> {
        let expression = expression(query.phrases);
        if expression.is_empty() {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(query.limit.get()).map_err(|_| Error::LimitOutOfRange)?;
        let mut statement = self.connection.prepare_cached(
            "SELECT rowid FROM documents_fts WHERE documents_fts MATCH ?1
             ORDER BY bm25(documents_fts, 10.0, 1.0, 4.0, 1.0, 1.0), rowid ASC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![expression, limit], |row| {
            let id: i64 = row.get(0)?;
            u64::try_from(id)
                .map(MessageId)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, id))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    fn visit_matches(
        &self,
        phrases: &[&str],
        mut visitor: impl FnMut(MessageId) -> bool,
    ) -> Result<()> {
        let expression = expression(phrases);
        if expression.is_empty() {
            return Ok(());
        }
        // One statement holds a consistent SQLite read snapshot for the walk.
        let mut statement = self.connection.prepare_cached(
            "SELECT rowid FROM documents_fts WHERE documents_fts MATCH ?1 ORDER BY rowid",
        )?;
        let mut rows = statement.query([expression])?;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let id = u64::try_from(id)
                .map(MessageId)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(0, id))?;
            if !visitor(id) {
                break;
            }
        }
        Ok(())
    }
}

fn expression(phrases: &[&str]) -> String {
    phrases
        .iter()
        .map(|phrase| phrase.trim())
        .filter(|phrase| !phrase.is_empty())
        .map(|phrase| format!("\"{}\"", phrase.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

fn sqlite_id(id: MessageId) -> Result<i64> {
    i64::try_from(id.0).map_err(|_| Error::MessageIdOutOfRange(id))
}
