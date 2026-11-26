use {
    crate::{Archive, Result},
    serde::Serialize,
};

/// A person the account exchanges mail with, in both directions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Correspondent {
    pub address: String,
    pub name: String,
    pub sent: u64,
    pub received: u64,
    pub last: i64,
}

// Own addresses are account names and configured identities. A person counts
// only after at least one message from an own address to them, so newsletters
// and one-way senders stay out. Junk and Trash copies are ignored.
const SQL: &str = "WITH own(address) AS (
        SELECT lower(account) FROM messages GROUP BY account
        UNION SELECT lower(address) FROM identities),
    exchanged(message_id, person, outbound) AS (
        SELECT f.message_id, r.address, 1 FROM addresses f JOIN addresses r ON r.message_id=f.message_id
        WHERE f.role='from' AND r.role IN ('to','cc')
        AND lower(f.address) IN (SELECT address FROM own) AND lower(r.address) NOT IN (SELECT address FROM own)
        UNION
        SELECT f.message_id, f.address, 0 FROM addresses f JOIN addresses r ON r.message_id=f.message_id
        WHERE f.role='from' AND r.role IN ('to','cc')
        AND lower(r.address) IN (SELECT address FROM own) AND lower(f.address) NOT IN (SELECT address FROM own))
    SELECT e.person,
        COALESCE((SELECT m2.sender FROM addresses a2 JOIN messages m2 ON m2.id=a2.message_id
            WHERE a2.address=e.person AND a2.role='from' ORDER BY m2.date DESC LIMIT 1), e.person),
        SUM(e.outbound), SUM(1-e.outbound), MAX(m.date)
    FROM exchanged e JOIN messages m ON m.id=e.message_id
    WHERE EXISTS(SELECT 1 FROM occurrences o WHERE o.message_id=m.id)
    AND NOT EXISTS(SELECT 1 FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id
        WHERE o.message_id=m.id AND COALESCE(b.user_role,b.server_role,
        CASE WHEN lower(b.name) IN ('junk','spam') THEN 'junk' WHEN lower(b.name) IN ('trash','deleted items') THEN 'trash' END) IN ('junk','trash'))
    GROUP BY e.person HAVING SUM(e.outbound)>0
    ORDER BY COUNT(*) DESC, MAX(m.date) DESC, e.person LIMIT ?1";

impl Archive {
    /// People most often exchanged with, by message count then recency.
    /// Reads metadata only; callers run it on demand, not per snapshot.
    pub fn frequent_correspondents(&self, limit: usize) -> Result<Vec<Correspondent>> {
        let mut statement = self.connection.prepare(SQL)?;
        let rows = statement.query_map([limit.clamp(1, 100) as i64], |row| {
            let address: String = row.get(0)?;
            let name: String = row.get(1)?;
            Ok(Correspondent {
                name: if name.trim().is_empty() {
                    address.clone()
                } else {
                    name
                },
                address,
                sent: row.get::<_, i64>(2)? as u64,
                received: row.get::<_, i64>(3)? as u64,
                last: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
