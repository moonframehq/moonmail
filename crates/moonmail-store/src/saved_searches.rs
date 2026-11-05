use {
    crate::{Archive, Error, Result},
    moonmail_core::MailPredicate,
    rusqlite::{Connection, OptionalExtension, params},
    serde::Serialize,
};

#[derive(Clone, Serialize)]
pub struct SavedSearch {
    pub id: i64,
    pub name: String,
    pub source: String,
    pub predicate: MailPredicate,
    pub sources: Vec<i64>,
}

fn row(row: &rusqlite::Row<'_>) -> rusqlite::Result<(i64, String, String, String)> {
    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
}
fn decode((id, name, source, json): (i64, String, String, String)) -> Result<SavedSearch> {
    let predicate = serde_json::from_str(&json)?;
    crate::predicates::compile(&predicate)?;
    Ok(SavedSearch {
        id,
        name,
        source,
        predicate,
        sources: Vec::new(),
    })
}
impl Archive {
    /// Saving an existing name explicitly replaces that saved query. This stores
    /// no message IDs or actions: every opening evaluates current live metadata.
    pub fn save_search(
        &mut self,
        name: &str,
        source: &str,
        predicate: &MailPredicate,
    ) -> Result<i64> {
        self.save_search_from(name, source, predicate, &[])
    }

    pub fn save_search_from(
        &mut self,
        name: &str,
        source: &str,
        predicate: &MailPredicate,
        sources: &[i64],
    ) -> Result<i64> {
        if name.trim().is_empty()
            || name.len() > 100
            || name.chars().any(char::is_control)
            || source.trim().is_empty()
            || source.len() > 4096
        {
            return Err(Error::Invalid("Invalid saved search name or query"));
        }
        crate::predicates::compile(predicate)?;
        if sources.len() > 100 {
            return Err(Error::Invalid("Too many saved search sources"));
        }
        let transaction = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM saved_searches WHERE name=?1",
                [name],
                |row| row.get(0),
            )
            .optional()?;
        let count: i64 =
            transaction.query_row("SELECT COUNT(*) FROM saved_searches", [], |row| row.get(0))?;
        if existing.is_none() && count >= 100 {
            return Err(Error::Invalid("At most 100 saved searches"));
        }
        let id=transaction.query_row("INSERT INTO saved_searches(name,source,predicate) VALUES (?1,?2,?3) ON CONFLICT(name) DO UPDATE SET source=excluded.source,predicate=excluded.predicate RETURNING id",params![name,source,serde_json::to_string(predicate)?],|row|row.get(0))?;
        transaction.execute("DELETE FROM search_sources WHERE search_id=?1", [id])?;
        for source in sources {
            if *source == id {
                return Err(Error::Invalid("A saved search cannot be its own source"));
            }
            let exists: bool = transaction.query_row(
                "SELECT EXISTS(SELECT 1 FROM saved_searches WHERE id=?1)",
                [source],
                |row| row.get(0),
            )?;
            if !exists {
                return Err(Error::Invalid("Saved search source no longer exists"));
            }
            transaction.execute(
                "INSERT OR IGNORE INTO search_sources VALUES (?1,?2)",
                params![id, source],
            )?;
        }
        // Validate descendants too: changing a parent must never leave a child
        // cyclic or too complex to open. The entire edit rolls back on failure.
        let searches = list(&transaction)?;
        for search in &searches {
            crate::search_sources::resolve(search.id, &searches)?;
        }
        transaction.commit()?;
        Ok(id)
    }
    pub fn saved_searches(&self) -> Result<Vec<SavedSearch>> {
        let transaction = self.connection.unchecked_transaction()?;
        let searches = list(&transaction)?;
        transaction.commit()?;
        Ok(searches)
    }
    pub fn saved_search(&self, id: i64) -> Result<SavedSearch> {
        self.saved_searches()?
            .into_iter()
            .find(|search| search.id == id)
            .ok_or(Error::NotFound)
    }
    pub fn resolve_search(&self, id: i64) -> Result<(SavedSearch, MailPredicate)> {
        let searches = self.saved_searches()?;
        let predicate = crate::search_sources::resolve(id, &searches)?;
        let search = searches
            .into_iter()
            .find(|search| search.id == id)
            .ok_or(Error::NotFound)?;
        Ok((search, predicate))
    }
    pub fn delete_search(&self, id: i64) -> Result<()> {
        let used: bool = self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM search_sources WHERE source_id=?1)",
            [id],
            |row| row.get(0),
        )?;
        if used {
            return Err(Error::Invalid(
                "This search is a source for another saved search; remove that dependency first",
            ));
        }
        if self
            .connection
            .execute("DELETE FROM saved_searches WHERE id=?1", [id])?
            != 1
        {
            return Err(Error::NotFound);
        }
        Ok(())
    }
}

fn list(connection: &Connection) -> Result<Vec<SavedSearch>> {
    let mut query=connection.prepare("SELECT id,name,source,predicate FROM saved_searches ORDER BY name COLLATE NOCASE,id LIMIT 100")?;
    let mut searches = query
        .query_map([], row)?
        .map(|result| decode(result?))
        .collect::<Result<Vec<_>>>()?;
    let mut query = connection
        .prepare("SELECT source_id FROM search_sources WHERE search_id=?1 ORDER BY source_id")?;
    for search in &mut searches {
        search.sources = query
            .query_map([search.id], |row| row.get(0))?
            .collect::<rusqlite::Result<_>>()?;
    }
    Ok(searches)
}
