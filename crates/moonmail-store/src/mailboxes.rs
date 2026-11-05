use {
    crate::{Archive, Error, Result},
    moonmail_core::MailboxRole,
    rusqlite::{OptionalExtension, params},
};

impl Archive {
    pub fn mailbox_server_role(&self, id: i64, role: Option<MailboxRole>) -> Result<()> {
        self.connection.execute(
            "UPDATE mailboxes SET server_role=?2 WHERE id=?1",
            params![id, role.map(MailboxRole::as_str)],
        )?;
        Ok(())
    }

    /// One explicit override per account/role. A later LIST cannot overwrite it.
    pub fn set_mailbox_role(&mut self, id: i64, role: Option<MailboxRole>) -> Result<()> {
        let transaction = self.connection.transaction()?;
        let account: String = transaction.query_row(
            "SELECT account FROM mailboxes WHERE id=?1 AND active=1 AND selectable=1",
            [id],
            |row| row.get(0),
        )?;
        if let Some(role) = role {
            transaction.execute(
                "UPDATE mailboxes SET user_role=NULL WHERE account=?1 AND user_role=?2",
                params![account, role.as_str()],
            )?;
        }
        transaction.execute(
            "UPDATE mailboxes SET user_role=?2 WHERE id=?1",
            params![id, role.map(MailboxRole::as_str)],
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Prefer the user's explicit role mapping. Never guess a destructive
    /// destination from a translated/display folder name or an ambiguous role.
    pub fn role_mailbox(&self, account: &str, role: MailboxRole) -> Result<i64> {
        let explicit = self
            .connection
            .query_row(
                "SELECT id,active AND selectable FROM mailboxes WHERE account=?1 AND user_role=?2",
                params![account, role.as_str()],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?;
        if let Some((id, active)) = explicit {
            return if active {
                Ok(id)
            } else {
                Err(Error::Invalid(
                    "The configured role folder is unavailable; choose a new folder override",
                ))
            };
        }
        let mut statement = self.connection.prepare(
            "SELECT id FROM mailboxes WHERE account=?1 AND active=1 AND selectable=1
             AND user_role IS NULL AND server_role=?2 ORDER BY id LIMIT 2",
        )?;
        let ids = statement
            .query_map(params![account, role.as_str()], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let Some(id) = ids.first().copied() else {
            return Err(Error::Invalid(
                "Choose a folder role in Move & folders first",
            ));
        };
        if ids.len() > 1 {
            return Err(Error::Invalid(
                "Multiple folders advertise this role; choose an explicit folder in Move & folders",
            ));
        }
        Ok(id)
    }

    /// Apply a complete, successful remote LIST to one account's mailbox
    /// namespace. Never call with a partial list after a protocol failure.
    /// Missing folders leave navigation; their occurrences and queued remote
    /// flags retire atomically. Messages, bodies and immutable blobs remain.
    /// Other accounts (including the separate local-import account) are untouched.
    pub fn reconcile_mailboxes<'a>(
        &mut self,
        account: &str,
        names: impl IntoIterator<Item = &'a str>,
    ) -> Result<()> {
        let transaction = self.connection.transaction()?;
        transaction.execute_batch(
            "CREATE TEMP TABLE IF NOT EXISTS listed_mailboxes(name TEXT PRIMARY KEY);
             DELETE FROM listed_mailboxes;",
        )?;
        for name in names {
            transaction.execute("INSERT INTO listed_mailboxes VALUES (?1)", [name])?;
            transaction.execute(
                "INSERT INTO mailboxes(account,name) VALUES (?1,?2) ON CONFLICT DO NOTHING",
                params![account, name],
            )?;
        }
        transaction.execute(
            "UPDATE mailboxes SET active=(name IN (SELECT name FROM listed_mailboxes)) WHERE account=?1",
            [account],
        )?;
        transaction.execute(
            "DELETE FROM flag_operations WHERE mailbox_id IN (SELECT id FROM mailboxes WHERE account=?1 AND active=0)",
            [account],
        )?;
        transaction.execute(
            "DELETE FROM occurrences WHERE mailbox_id IN (SELECT id FROM mailboxes WHERE account=?1 AND active=0)",
            [account],
        )?;
        transaction.commit()?;
        Ok(())
    }
}
