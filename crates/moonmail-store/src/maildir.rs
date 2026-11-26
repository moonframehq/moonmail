use {
    crate::{Archive, Error, Result},
    rusqlite::params,
    std::path::Path,
};

pub struct MaildirAccount {
    pub account: String,
    pub root: String,
    pub device: String,
}

pub struct MaildirLocation {
    pub stem: String,
    pub fingerprint: String,
}

pub struct MaildirFile {
    pub id: i64,
    pub blob: String,
    pub mailbox: i64,
    pub location: MaildirLocation,
    pub seen: bool,
    pub starred: bool,
    pub want_seen: Option<bool>,
    pub want_star: Option<bool>,
}

pub(crate) fn check_folder_editable(db: &rusqlite::Connection, account: &str) -> Result<()> {
    if db.query_row(
        "SELECT EXISTS(SELECT 1 FROM maildir_accounts WHERE account=?1)",
        [account],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(Error::Invalid(
            "Use Mutt or mbsync for Maildir moves, copies and folder edits",
        ));
    }
    Ok(())
}

impl Archive {
    pub fn register_maildir(&mut self, root: &str, device: &str) -> Result<()> {
        let tx = self.connection.transaction()?;
        {
            let mut q = tx.prepare("SELECT root,device FROM maildir_accounts")?;
            let mut rows = q.query([])?;
            while let Some(row) = rows.next()? {
                let existing: String = row.get(0)?;
                let same_directory = row.get::<_, String>(1)? == device;
                if existing == root && same_directory {
                    return Ok(());
                }
                if same_directory
                    || Path::new(&existing).starts_with(root)
                    || Path::new(root).starts_with(existing)
                {
                    return Err(Error::Invalid(
                        "Maildir overlaps an existing account or its root was replaced",
                    ));
                }
            }
        }
        tx.execute(
            "INSERT INTO maildir_accounts VALUES (?1,?2,?3)",
            params![format!("Maildir {root}"), root, device],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn maildir_accounts(&self) -> Result<Vec<MaildirAccount>> {
        let mut q = self
            .connection
            .prepare("SELECT account,root,device FROM maildir_accounts ORDER BY root")?;
        Ok(q.query_map([], |r| {
            Ok(MaildirAccount {
                account: r.get(0)?,
                root: r.get(1)?,
                device: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }

    pub fn maildir_folders(
        &self,
        account: &str,
    ) -> Result<std::collections::HashMap<String, (i64, String)>> {
        let mut q = self.connection.prepare(
            "SELECT b.name,b.id,f.relative
             FROM mailboxes b JOIN maildir_folders f ON f.mailbox=b.id
             WHERE b.account=?1 AND b.active",
        )?;
        Ok(
            q.query_map([account], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
                .collect::<rusqlite::Result<_>>()?,
        )
    }

    pub fn ensure_maildir_folder(
        &mut self,
        account: &str,
        name: &str,
        relative: &str,
    ) -> Result<i64> {
        let tx = self.connection.transaction()?;
        let id = tx.query_row(
            "INSERT INTO mailboxes(account,name) VALUES (?1,?2)
             ON CONFLICT(account,name) DO UPDATE SET active=1 RETURNING id",
            params![account, name],
            |r| r.get(0),
        )?;
        tx.execute(
            "INSERT INTO maildir_folders VALUES (?1,?2)
             ON CONFLICT(mailbox) DO UPDATE SET relative=excluded.relative",
            params![id, relative],
        )?;
        tx.commit()?;
        Ok(id)
    }

    pub fn maildir_files(&self, account: &str) -> Result<Vec<MaildirFile>> {
        let mut q=self.connection.prepare("SELECT f.message,f.mailbox,f.stem,f.fingerprint,o.seen,o.flagged,f.want_seen,f.want_star,m.blob
             FROM mailboxes b JOIN maildir_files f ON f.mailbox=b.id
             JOIN messages m ON m.id=f.message
             JOIN occurrences o ON o.mailbox_id=b.id AND o.message_id=f.message
             WHERE b.account=?1")?;
        Ok(q.query_map([account], |r| {
            Ok(MaildirFile {
                id: r.get(0)?,
                mailbox: r.get(1)?,
                blob: r.get(8)?,
                location: MaildirLocation {
                    stem: r.get(2)?,
                    fingerprint: r.get(3)?,
                },
                seen: r.get(4)?,
                starred: r.get(5)?,
                want_seen: r.get(6)?,
                want_star: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }

    /// Acknowledge only the intent applied by this scan. A newer edit remains
    /// both visible in the occurrence and queued for the next rename.
    pub fn observe_maildir(
        &mut self,
        old: &MaildirFile,
        mailbox: i64,
        location: &MaildirLocation,
        seen: bool,
        star: bool,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE maildir_files SET mailbox=?2,stem=?3,fingerprint=?6,
             want_seen=CASE WHEN want_seen IS ?4 THEN NULL ELSE want_seen END,
             want_star=CASE WHEN want_star IS ?5 THEN NULL ELSE want_star END
             WHERE message=?1",
            params![
                old.id,
                mailbox,
                location.stem,
                old.want_seen,
                old.want_star,
                location.fingerprint
            ],
        )?;
        if old.mailbox != mailbox {
            tx.execute("INSERT INTO occurrences(mailbox_id,message_id,epoch,seen,flagged)
                 SELECT ?2,message_id,0,seen,flagged FROM occurrences WHERE mailbox_id=?3 AND message_id=?1",params![old.id,mailbox,old.mailbox])?;
            tx.execute("INSERT INTO occurrence_keywords
                 SELECT ?2,message_id,keyword FROM occurrence_keywords WHERE mailbox_id=?3 AND message_id=?1",params![old.id,mailbox,old.mailbox])?;
            tx.execute(
                "DELETE FROM occurrences WHERE mailbox_id=?2 AND message_id=?1",
                params![old.id, old.mailbox],
            )?;
            crate::rules::on_entry(&tx, mailbox, moonmail_core::MessageId(old.id as u64))?;
        }
        tx.execute(
            "UPDATE occurrences SET (seen,flagged)=(
                 SELECT COALESCE(want_seen,?3),COALESCE(want_star,?4)
                 FROM maildir_files WHERE message=?1)
             WHERE message_id=?1 AND mailbox_id=?2",
            params![old.id, mailbox, seen, star],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn prune_maildir(&mut self, files: &[&MaildirFile], folders: &[i64]) -> Result<()> {
        let tx = self.connection.transaction()?;
        for file in files {
            tx.execute(
                "DELETE FROM occurrences WHERE mailbox_id=?1 AND message_id=?2",
                params![file.mailbox, file.id],
            )?;
            tx.execute("DELETE FROM maildir_files WHERE message=?1", [file.id])?;
        }
        for id in folders {
            tx.execute("UPDATE mailboxes SET active=0 WHERE id=?1", [id])?;
        }
        tx.commit()?;
        Ok(())
    }
}
