use {
    crate::{Archive, Error, SearchError},
    moonmail_core::SearchIndex,
};

impl Archive {
    /// Atomically replace this connection's temporary match set. The index is
    /// streamed into a disk-backed table with a bounded page cache; neither raw
    /// messages nor an unbounded Vec of IDs are needed. On failure the previous
    /// set remains usable. No authoritative metadata or checkpoint is changed.
    ///
    /// Reprepare after the index changes. Folder/live/flag filters are evaluated
    /// by `page`, not frozen here. Call on the connection-owning worker.
    pub fn prepare_search<I: SearchIndex>(
        &mut self,
        index: &I,
        phrases: &[&str],
    ) -> Result<(), SearchError<I::Error>> {
        let transaction = self.connection.transaction().map_err(Error::from)?;
        transaction
            .execute("DELETE FROM search_matches", [])
            .map_err(Error::from)?;
        let mut insert = transaction
            .prepare("INSERT INTO search_matches(id) VALUES (?1)")
            .map_err(Error::from)?;
        let mut failure = None;
        index
            .visit_matches(phrases, |id| {
                let result = i64::try_from(id.0)
                    .map_err(|_| Error::Invalid("search message ID exceeds SQLite range"))
                    .and_then(|id| insert.execute([id]).map_err(Error::from));
                match result {
                    Ok(_) => true,
                    Err(error) => {
                        failure = Some(error);
                        false
                    },
                }
            })
            .map_err(SearchError::Index)?;
        if let Some(error) = failure {
            return Err(error.into());
        }
        drop(insert);
        transaction.commit().map_err(Error::from)?;
        self.search_prepared = true;
        self.search_predicate = None;
        Ok(())
    }
}
