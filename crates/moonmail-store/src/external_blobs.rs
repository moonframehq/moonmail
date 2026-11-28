//! Originals that stay in a msgvault archive instead of a Moonmail file.
//! Moonmail keeps the metadata and search index; opening a message reads its
//! MIME from msgvault's `message_raw`, read-only, and checks its SHA-256.
use {
    crate::{Archive, BlobError, Result},
    rusqlite::{Connection, OpenFlags, OptionalExtension, params},
    std::{io::Read, path::Path, time::Duration},
};

const LIMIT: u64 = 64 * 1024 * 1024;

fn open(path: &Path) -> rusqlite::Result<Connection> {
    let db = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    db.busy_timeout(Duration::from_secs(10))?;
    Ok(db)
}

/// The original for `hash` from its msgvault archive, if one is recorded.
pub(crate) fn read(root: &Path, hash: &str) -> std::result::Result<Option<Vec<u8>>, BlobError> {
    let archive = root.join("archive.sqlite3");
    if !archive.exists() {
        return Ok(None);
    }
    let Some((database, row)) = open(&archive)?
        .query_row(
            "SELECT database,row FROM external_blobs WHERE hash=?1",
            [hash],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let Some((data, compression)) = open(Path::new(&database))?
        .query_row(
            "SELECT raw_data,compression FROM message_raw WHERE message_id=?1 AND raw_format='mime'",
            [row],
            |r| Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()?
    else {
        return Err(BlobError::ExternalMissing);
    };
    let bytes = match compression.as_deref() {
        None | Some("" | "none") => data,
        Some("zlib") => {
            let mut out = Vec::new();
            flate2::read::ZlibDecoder::new(data.as_slice())
                .take(LIMIT + 1)
                .read_to_end(&mut out)?;
            out
        },
        Some(_) => return Err(BlobError::ExternalMissing),
    };
    Ok(Some(bytes))
}

impl Archive {
    /// Record where msgvault keeps an original before a message refers to it,
    /// so every stored message stays readable.
    pub fn reference_external(&self, hash: &str, database: &str, row: i64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO external_blobs VALUES (?1,?2,?3) ON CONFLICT(hash) DO NOTHING",
            params![hash, database, row],
        )?;
        Ok(())
    }

    /// Originals kept by msgvault whose Moonmail file only msgvault mirrors
    /// use, after `after` in hash order. Their files can be released.
    pub fn releasable_blobs(&self, after: &str, limit: usize) -> Result<Vec<String>> {
        let mut q = self.connection.prepare(
            "SELECT e.hash FROM external_blobs e WHERE e.hash>?1 AND NOT EXISTS(
                 SELECT 1 FROM messages m WHERE m.blob=e.hash
                 AND m.account NOT IN (SELECT account FROM msgvault_accounts))
             ORDER BY e.hash LIMIT ?2",
        )?;
        Ok(q.query_map(params![after, limit as i64], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?)
    }
}
