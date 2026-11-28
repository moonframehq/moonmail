use {
    crate::{Archive, Result},
    rusqlite::params,
};

pub struct JmapDeletion {
    pub id: i64,
    pub email: String,
    pub folder: String,
    pub role: String,
    pub mailbox: Option<i64>,
    pub user_role: bool,
}

impl Archive {
    pub fn is_jmap_folder(&self, mailbox: i64) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM jmap_folders WHERE mailbox=?1)",
            [mailbox],
            |r| r.get(0),
        )?)
    }
    pub fn claim_jmap_deletions(&mut self, page: &[JmapDeletion]) -> Result<Vec<bool>> {
        let tx = self.connection.transaction()?;
        let claimed = {
            let mut q=tx.prepare_cached("UPDATE expunge_operations SET state='unknown' WHERE id=?1 AND epoch=0 AND state='queued'")?;
            page.iter()
                .map(|op| q.execute([op.id]).map(|n| n == 1))
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        tx.commit()?;
        Ok(claimed)
    }
    /// A page contains targets for the same stable remote folder and confirmed role.
    pub fn jmap_deletions(&self, account: &str, limit: u16) -> Result<Vec<JmapDeletion>> {
        let mut q = self.connection.prepare_cached(
            "SELECT d.id,e.remote,d.remote_folder,d.confirmed_role,b.id,b.user_role IS NOT NULL
             FROM expunge_operations d JOIN jmap_emails e ON e.account=d.account AND e.uid=d.uid
             LEFT JOIN jmap_folders f ON f.account=d.account AND f.remote=d.remote_folder
             LEFT JOIN mailboxes b ON b.id=f.mailbox AND b.active AND b.epoch=0
             WHERE d.account=?1 AND d.epoch=0 AND d.state='queued' AND (d.remote_folder,d.confirmed_role)=(
                SELECT remote_folder,confirmed_role FROM expunge_operations WHERE account=?1 AND epoch=0 AND state='queued' ORDER BY id LIMIT 1)
             ORDER BY d.id LIMIT ?2",
        )?;
        Ok(q.query_map(params![account, limit], |r| {
            Ok(JmapDeletion {
                id: r.get(0)?,
                email: r.get(1)?,
                folder: r.get(2)?,
                role: r.get(3)?,
                mailbox: r.get(4)?,
                user_role: r.get(5)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?)
    }
}
