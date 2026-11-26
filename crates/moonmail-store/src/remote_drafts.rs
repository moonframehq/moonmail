use {
    crate::{Archive, Draft, Result},
    rusqlite::{OptionalExtension, params},
};

pub struct RemoteDraftSource {
    pub uid: u32,
    pub blob: String,
    pub body: String,
    pub upload: Option<String>,
}

impl Archive {
    /// Read one candidate. The upload claim rejects intervening edits.
    pub fn dirty_draft(&mut self, account: &str, after: i64) -> Result<Option<(Draft, i64)>> {
        let candidate = self
            .connection
            .query_row(
                "SELECT id,updated
                 FROM drafts
                 WHERE account=?1 AND dirty
                 AND updated>?2
                 AND NOT EXISTS(SELECT 1
                 FROM draft_uploads
                 WHERE draft_id=drafts.id)
                 ORDER BY updated LIMIT 1",
                params![account, after],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?;
        // The claim below verifies the revision again after MIME construction.
        match candidate {
            Some((id, revision)) => Ok(self.draft(&id)?.map(|draft| (draft, revision))),
            None => Ok(None),
        }
    }
    pub fn claim_draft_upload(
        &self,
        draft: &Draft,
        revision: i64,
        mailbox: &str,
        message_id: &str,
    ) -> Result<bool> {
        Ok(self.connection.execute(
            "INSERT OR IGNORE INTO draft_uploads(draft_id,account,mailbox,revision,message_id,subject)
                 SELECT id,account,?3,updated,?4,subject
                 FROM drafts
                 WHERE id=?1
                 AND updated=?2
                 AND account=?5 AND dirty",
            params![draft.id, revision, mailbox, message_id, draft.account],
        )? == 1)
    }

    /// Install the acknowledged replacement and retire the previous copy in a
    /// single commit. A concurrent edit stays dirty. A discarded or reassigned
    /// draft records cleanup for the newly discovered copy as well.
    pub fn accept_draft_upload(
        &mut self,
        account: &str,
        mailbox: &str,
        epoch: u32,
        uid: u32,
        message_id: &str,
    ) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let Some((id,revision))=tx.query_row("SELECT draft_id,revision FROM draft_uploads WHERE account=?1 AND mailbox=?2 AND message_id=?3",params![account,mailbox,message_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?))).optional()? else {
            return Ok(false);
        };
        tx.execute("INSERT OR IGNORE INTO expunge_operations(account,mailbox,epoch,uid,draft_id)
                 SELECT account,remote_mailbox,remote_epoch,remote_uid,id
                 FROM drafts
                 WHERE id=?1
                 AND account=?2
                 AND remote_uid IS NOT NULL AND (remote_mailbox,remote_epoch,remote_uid)<>(?3,?4,?5)",params![id,account,mailbox,epoch,uid])?;
        if tx.execute("UPDATE drafts
                 SET dirty=(updated<>?6),remote_mailbox=?3,remote_epoch=?4,remote_uid=?5,remote_synced_at=unixepoch(),pending_remote_delete=EXISTS(SELECT 1
                 FROM expunge_operations
                 WHERE draft_id=?1
                 AND state NOT IN ('done','dismissed'))
                 WHERE id=?1
                 AND account=?2",params![id,account,mailbox,epoch,uid,revision])? == 0 {
            tx.execute("INSERT OR IGNORE INTO expunge_operations(account,mailbox,epoch,uid,draft_id) VALUES(?1,?2,?3,?4,?5)",params![account,mailbox,epoch,uid,id])?;
        }
        tx.execute(
            "DELETE FROM draft_uploads WHERE message_id=?1",
            [message_id],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Only the just-completed inventory may retire clean composer rows. A
    /// dirty local revision survives an external edit as a separate draft.
    pub fn retire_missing_drafts(
        &mut self,
        account: &str,
        folder: &str,
        mailbox: i64,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        // Clear UID before deleting: the server has already removed this copy.
        tx.execute(
            "UPDATE drafts
                 SET remote_uid=NULL
                 WHERE account=?1
                 AND remote_mailbox=?2
                 AND EXISTS(SELECT 1
                 FROM sync_scope c
                 WHERE c.complete AND c.mailbox=?3 AND (c.epoch<>remote_epoch
                 OR (remote_uid<=c.upper_uid
                 AND NOT EXISTS(SELECT 1
                 FROM sync_inventory s
                 WHERE s.uid=remote_uid))))",
            params![account, folder, mailbox],
        )?;
        tx.execute("DELETE FROM drafts WHERE account=?1 AND remote_mailbox=?2 AND remote_uid IS NULL AND dirty=0",params![account,folder])?;
        tx.commit()?;
        Ok(())
    }

    pub fn retry_draft_upload(&self, message_id: &str) -> Result<()> {
        self.connection.execute(
            "DELETE FROM draft_uploads WHERE message_id=?1",
            [message_id],
        )?;
        Ok(())
    }
    pub fn unsupported_draft_upload(&self, message_id: &str) -> Result<()> {
        self.connection.execute(
            "UPDATE draft_uploads SET state='unsupported' WHERE message_id=?1",
            [message_id],
        )?;
        Ok(())
    }

    pub fn unimported_draft(
        &self,
        account: &str,
        folder: &str,
        mailbox: i64,
        epoch: u32,
        after: u32,
    ) -> Result<Option<RemoteDraftSource>> {
        Ok(self.connection.query_row(
            "SELECT o.uid,m.blob,b.text,(
                SELECT u.message_id FROM draft_uploads u WHERE u.account=?1 AND u.mailbox=?2
                  AND u.message_id=m.rfc_id AND 1=(SELECT COUNT(*) FROM occurrences p
                    JOIN messages n ON n.id=p.message_id WHERE p.mailbox_id=?3
                    AND p.epoch=?4 AND n.rfc_id=u.message_id))
             FROM occurrences o JOIN messages m ON m.id=o.message_id JOIN bodies b ON b.message_id=m.id
             WHERE o.mailbox_id=?3 AND o.epoch=?4 AND o.uid>?5
               AND NOT EXISTS(SELECT 1 FROM drafts WHERE account=?1 AND remote_mailbox=?2 AND remote_epoch=?4 AND remote_uid=o.uid)
               AND NOT EXISTS(SELECT 1 FROM expunge_operations WHERE account=?1 AND mailbox=?2 AND epoch=?4 AND uid=o.uid)
             ORDER BY o.uid LIMIT 1",params![account,folder,mailbox,epoch,after],|r|Ok(RemoteDraftSource{uid:r.get(0)?,blob:r.get(1)?,body:r.get(2)?,upload:r.get(3)?})).optional()?)
    }
    pub fn import_remote_draft(
        &mut self,
        draft: &Draft,
        folder: &str,
        epoch: u32,
        uid: u32,
    ) -> Result<()> {
        let tx = self.connection.transaction()?;
        let known: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1
                 FROM drafts
                 WHERE account=?1
                 AND remote_mailbox=?2 AND remote_epoch=?3
                 AND remote_uid=?4
                 UNION ALL
                 SELECT 1
                 FROM expunge_operations
                 WHERE account=?1 AND mailbox=?2 AND epoch=?3 AND uid=?4)",
            params![draft.account, folder, epoch, uid],
            |r| r.get(0),
        )?;
        if !known {
            Self::persist_draft(&tx, draft)?;
            tx.execute("UPDATE drafts
                 SET dirty=0,remote_mailbox=?2,remote_epoch=?3,remote_uid=?4,remote_synced_at=unixepoch()
                 WHERE id=?1",params![draft.id,folder,epoch,uid])?;
        }
        tx.commit()?;
        Ok(())
    }
}
