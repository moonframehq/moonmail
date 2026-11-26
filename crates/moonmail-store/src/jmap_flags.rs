use {
    crate::{Archive, Result},
    rusqlite::params,
};

pub struct JmapFlag {
    pub id: i64,
    mailbox: i64,
    uid: u32,
    pub remote: String,
    pub kind: String,
    pub value: bool,
}

impl Archive {
    /// Read a bounded page of durable JMAP intents in journal order.
    pub fn jmap_flags(&self, account: &str, after: i64, limit: u16) -> Result<Vec<JmapFlag>> {
        let mut query = self.connection.prepare_cached(
            "SELECT f.id,f.mailbox_id,f.uid,e.remote,f.kind,f.value
             FROM flag_operations f JOIN mailboxes b ON b.id=f.mailbox_id
             LEFT JOIN jmap_emails e ON e.uid=f.uid AND e.account=b.account
             WHERE f.id>?1 AND f.epoch=0 AND b.account=?2 ORDER BY f.id LIMIT ?3",
        )?;
        Ok(query
            .query_map(params![after, account, limit], |r| {
                Ok(JmapFlag {
                    id: r.get(0)?,
                    mailbox: r.get(1)?,
                    uid: r.get(2)?,
                    remote: r.get(3)?,
                    kind: r.get(4)?,
                    value: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    /// Match both identity and value in case another writer changed an intent in flight.
    pub fn acknowledge_jmap_flags(&mut self, flags: &[&JmapFlag]) -> Result<()> {
        let tx = self.connection.transaction()?;
        {
            let mut delete = tx.prepare_cached(
                "DELETE FROM flag_operations WHERE id=?1 AND mailbox_id=?2 AND epoch=0
                 AND uid=?3 AND kind=?4 AND value=?5",
            )?;
            for f in flags {
                delete.execute(params![f.id, f.mailbox, f.uid, f.kind, f.value])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
}
