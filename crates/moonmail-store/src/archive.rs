use {
    crate::{Error, IngestError, Result},
    moonmail_core::{BlobStore, MessageId, MimeParser, ParsedMessage, SearchDocument},
    rusqlite::{Connection, OptionalExtension, params},
    std::{path::Path, time::Duration},
};

pub struct Archive {
    pub(crate) connection: Connection,
}

pub struct IncomingMessage<'a> {
    pub account: &'a str,
    pub mailbox: i64,
    pub origin: &'a str,
    pub epoch: u32,
    pub uid: Option<u32>,
    pub seen: bool,
    pub starred: bool,
    pub received_at: i64,
    pub raw: &'a [u8],
}

pub struct IndexWork {
    pub sequence: i64,
    pub id: MessageId,
    pub document: SearchDocument,
}

impl Archive {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(root.as_ref())?;
        let mut connection = Connection::open(root.as_ref().join("archive.sqlite3"))?;
        connection.busy_timeout(Duration::from_secs(10))?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        let transaction = connection.transaction()?;
        let version: i64 =
            transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => transaction.execute_batch(include_str!("schema.sql"))?,
            1 | 2 => {},
            version => return Err(Error::Schema(version)),
        }
        if version < 2 {
            transaction.execute_batch(
                "ALTER TABLE mailboxes ADD COLUMN delimiter TEXT DEFAULT '/';
             ALTER TABLE mailboxes ADD COLUMN selectable INTEGER NOT NULL DEFAULT 1;
             CREATE INDEX IF NOT EXISTS messages_sender ON messages(sender COLLATE NOCASE,id);
             CREATE INDEX IF NOT EXISTS messages_subject ON messages(subject COLLATE NOCASE,id);
             PRAGMA user_version=2;",
            )?;
        }
        transaction.commit()?;
        Ok(Self { connection })
    }

    pub fn mailbox(&self, account: &str, name: &str) -> Result<i64> {
        self.connection.execute(
            "INSERT INTO mailboxes(account,name) VALUES (?1,?2) ON CONFLICT DO NOTHING",
            params![account, name],
        )?;
        Ok(self.connection.query_row(
            "SELECT id FROM mailboxes WHERE account=?1 AND name=?2",
            params![account, name],
            |row| row.get(0),
        )?)
    }

    pub fn mailbox_hierarchy(
        &self,
        id: i64,
        delimiter: Option<&str>,
        selectable: bool,
    ) -> Result<()> {
        self.connection.execute(
            "UPDATE mailboxes SET delimiter=?2,selectable=?3 WHERE id=?1",
            params![id, delimiter, selectable],
        )?;
        Ok(())
    }

    pub fn ingest<P: MimeParser, B: BlobStore>(
        &mut self,
        parser: &P,
        blobs: &B,
        incoming: IncomingMessage<'_>,
    ) -> std::result::Result<MessageId, IngestError<P::Error, B::Error>> {
        if incoming.raw.len() > 64 * 1024 * 1024 {
            return Err(Error::Invalid("message exceeds 64 MiB limit").into());
        }
        let parsed = parser.parse(incoming.raw).map_err(IngestError::Parser)?;
        let hash = blobs.put(incoming.raw).map_err(IngestError::Blob)?;
        self.insert_parsed(incoming, parsed, &hash)
            .map_err(IngestError::Store)
    }

    fn insert_parsed(
        &mut self,
        incoming: IncomingMessage<'_>,
        parsed: ParsedMessage,
        hash: &str,
    ) -> Result<MessageId> {
        let text = parsed.text_body.clone().unwrap_or_else(|| {
            parsed
                .html_body
                .as_deref()
                .and_then(|html| html2text::from_read(html.as_bytes(), 100).ok())
                .unwrap_or_default()
        });
        let snippet = text
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .chars()
            .take(180)
            .collect::<String>();
        let sender = parsed
            .from
            .first()
            .map(|sender| sender.name.as_deref().unwrap_or(&sender.address))
            .unwrap_or("Unknown sender");
        let address = parsed
            .from
            .first()
            .map(|sender| sender.address.as_str())
            .unwrap_or_default();
        let recipients = parsed
            .to
            .iter()
            .map(|recipient| recipient.address.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let thread_key = parsed
            .references
            .first()
            .or(parsed.in_reply_to.as_ref())
            .or(parsed.message_id.as_ref())
            .map(String::as_str)
            .unwrap_or(incoming.origin);
        let transaction = self.connection.transaction()?;
        let existing: Option<i64> = transaction
            .query_row(
                "SELECT id FROM messages WHERE account=?1 AND origin=?2",
                params![incoming.account, incoming.origin],
                |row| row.get(0),
            )
            .optional()?;
        let id = if let Some(id) = existing {
            id
        } else {
            transaction.execute(
                "INSERT INTO messages(account,origin,blob,subject,sender,address,recipients,date,snippet,rfc_id,thread_key) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                params![incoming.account,incoming.origin,hash,parsed.subject.as_deref().unwrap_or("(No subject)"),sender,address,recipients,parsed.date.unwrap_or(incoming.received_at),snippet,parsed.message_id,thread_key],
            )?;
            let id = transaction.last_insert_rowid();
            transaction.execute(
                "INSERT INTO bodies VALUES (?1,?2,?3)",
                params![id, text, parsed.html_body],
            )?;
            for (role, addresses) in [
                ("from", &parsed.from),
                ("to", &parsed.to),
                ("cc", &parsed.cc),
            ] {
                for address in addresses {
                    transaction.execute(
                        "INSERT OR IGNORE INTO addresses VALUES (?1,?2,?3)",
                        params![id, role, canonical_address(&address.address)],
                    )?;
                }
            }
            transaction.execute("INSERT INTO changes(message_id) VALUES (?1)", [id])?;
            id
        };
        transaction.execute("INSERT INTO occurrences(mailbox_id,message_id,epoch,uid,seen,flagged) VALUES (?1,?2,?3,?4,?5,?6) ON CONFLICT(mailbox_id,message_id) DO UPDATE SET seen=excluded.seen,flagged=excluded.flagged", params![incoming.mailbox,id,incoming.epoch,incoming.uid,incoming.seen,incoming.starred])?;
        transaction.commit()?;
        Ok(MessageId(id as u64))
    }

    pub fn index_work(&self, limit: usize) -> Result<Vec<IndexWork>> {
        let mut statement = self.connection.prepare_cached(
            "SELECT c.seq,m.id,m.subject,b.text,m.address,m.recipients FROM changes c JOIN messages m ON m.id=c.message_id JOIN bodies b ON b.message_id=m.id WHERE c.seq>(SELECT cursor FROM projection WHERE name='search') ORDER BY c.seq LIMIT ?1"
        )?;
        let rows = statement.query_map([limit.min(500) as i64], |row| {
            Ok(IndexWork {
                sequence: row.get(0)?,
                id: MessageId(row.get::<_, i64>(1)? as u64),
                document: SearchDocument {
                    subject: row.get(2)?,
                    body: row.get(3)?,
                    from: row.get(4)?,
                    to: row.get(5)?,
                    cc: String::new(),
                },
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn acknowledge_index(&self, sequence: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE projection SET cursor=MAX(cursor,?1) WHERE name='search'",
            [sequence],
        )?;
        Ok(())
    }

    pub fn reset_index(&self) -> Result<()> {
        self.connection
            .execute("UPDATE projection SET cursor=0 WHERE name='search'", [])?;
        Ok(())
    }

    /// The server's current UID set is authoritative only after SELECT succeeded.
    /// Old bytes stay archived, but invalid/expunged occurrences leave live views.
    pub fn reconcile(
        &mut self,
        mailbox: i64,
        epoch: u32,
        flags: &[(u32, bool, bool)],
    ) -> Result<Vec<u32>> {
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "DELETE FROM occurrences WHERE mailbox_id=?1 AND epoch<>?2",
            params![mailbox, epoch],
        )?;
        transaction.execute(
            "UPDATE mailboxes SET epoch=?2 WHERE id=?1",
            params![mailbox, epoch],
        )?;
        transaction.execute_batch("CREATE TEMP TABLE IF NOT EXISTS live_uids(uid INTEGER PRIMARY KEY); DELETE FROM live_uids;")?;
        for (uid, seen, flagged) in flags {
            transaction.execute("INSERT INTO live_uids VALUES (?1)", [uid])?;
            transaction.execute("UPDATE occurrences SET seen=CASE WHEN EXISTS(SELECT 1 FROM flag_operations WHERE mailbox_id=?1 AND epoch=?2 AND uid=?3 AND kind='seen') THEN seen ELSE ?4 END, flagged=CASE WHEN EXISTS(SELECT 1 FROM flag_operations WHERE mailbox_id=?1 AND epoch=?2 AND uid=?3 AND kind='flagged') THEN flagged ELSE ?5 END WHERE mailbox_id=?1 AND epoch=?2 AND uid=?3",params![mailbox,epoch,uid,seen,flagged])?;
        }
        transaction.execute("DELETE FROM occurrences WHERE mailbox_id=?1 AND uid IS NOT NULL AND uid NOT IN (SELECT uid FROM live_uids)",[mailbox])?;
        let known = {
            let mut statement = transaction.prepare(
                "SELECT uid FROM occurrences WHERE mailbox_id=?1 AND epoch=?2 AND uid IS NOT NULL",
            )?;
            statement
                .query_map(params![mailbox, epoch], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<u32>>>()?
        };
        transaction.commit()?;
        Ok(known)
    }

    pub fn total(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COUNT(*) FROM messages", [], |row| {
                row.get::<_, i64>(0)
            })? as u64)
    }
}

fn canonical_address(address: &str) -> String {
    match address.rsplit_once('@') {
        Some((local, domain)) => format!("{local}@{}", domain.to_ascii_lowercase()),
        None => address.to_owned(),
    }
}
