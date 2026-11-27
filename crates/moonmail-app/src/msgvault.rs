//! Mirror a msgvault archive (<https://github.com/wesm/msgvault>) as read-only
//! accounts, one per email source. msgvault's database is only ever opened
//! read-only; its labels become mailboxes and its read state follows along.
use {
    anyhow::{Context, Result},
    flate2::read::ZlibDecoder,
    moonmail_core::MailboxRole,
    moonmail_mime::StalwartParser,
    moonmail_store::{
        Archive, FileBlobs, IncomingMessage, IngestError, MsgvaultAccount, MsgvaultChange,
    },
    rusqlite::{Connection, OpenFlags, OptionalExtension},
    std::{
        collections::HashMap,
        fs::{self, File},
        io::Read,
        path::{Path, PathBuf},
        sync::mpsc,
        time::Duration,
    },
};

const LIMIT: u64 = 64 * 1024 * 1024;
const BATCH: i64 = 200;
const SEPARATOR: char = '\u{1f}';

/// msgvault's own lookup: `$MSGVAULT_HOME` or `~/.msgvault`, whose optional
/// `config.toml` may move the data directory with `[data] data_dir`.
pub fn locate() -> Option<PathBuf> {
    if std::env::var_os("MOONMAIL_MSGVAULT").is_some_and(|value| value == "0") {
        return None;
    }
    let home_dir = PathBuf::from(std::env::var_os("HOME")?);
    let expand = |path: &str| match path.strip_prefix("~/") {
        Some(rest) => home_dir.join(rest),
        None if path == "~" => home_dir.clone(),
        None => PathBuf::from(path),
    };
    let home = std::env::var("MSGVAULT_HOME")
        .ok()
        .filter(|value| !value.is_empty())
        .map_or_else(|| home_dir.join(".msgvault"), |value| expand(&value));
    let data = fs::read_to_string(home.join("config.toml"))
        .ok()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|config| {
            config
                .get("data")?
                .get("data_dir")?
                .as_str()
                .filter(|dir| !dir.trim().is_empty())
                .map(expand)
        })
        .unwrap_or(home);
    let database = data.join("msgvault.db");
    database.is_file().then_some(database)
}

/// Register every msgvault source holding email. Returns the new accounts.
pub fn detect(root: &Path, database: &Path) -> Result<Vec<String>> {
    let database = database.canonicalize()?;
    let vault = open(&database)?;
    let mut archive = Archive::open(root)?;
    let path = database.to_str().context("msgvault path must be UTF-8")?;
    let mut added = Vec::new();
    let mut q = vault.prepare(
        "SELECT id,identifier FROM sources s WHERE EXISTS(
             SELECT 1 FROM messages m WHERE m.source_id=s.id AND m.message_type='email')
         ORDER BY id",
    )?;
    for source in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (id, identifier) = source?;
        let account = format!("msgvault {}", address(&identifier));
        if archive.register_msgvault(&account, path, id)? {
            added.push(account);
        }
    }
    Ok(added)
}

/// `imaps://user@example.com@imap.example.com:993` names its login, not the server.
fn address(identifier: &str) -> &str {
    match identifier.split_once("://") {
        Some((_, rest)) => rest.rsplit_once('@').map_or(rest, |(user, _)| user),
        None => identifier,
    }
}

fn open(database: &Path) -> Result<Connection> {
    let vault = Connection::open_with_flags(
        database,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("Opening msgvault at {}", database.display()))?;
    vault.busy_timeout(Duration::from_secs(10))?;
    vault.pragma_update(None, "query_only", true)?;
    Ok(vault)
}

fn role(name: &str, system: Option<&str>) -> Option<MailboxRole> {
    let name = name.to_ascii_lowercase();
    let name = name.strip_prefix("[gmail]/").unwrap_or(&name);
    Some(match system.unwrap_or(name) {
        "inbox" => MailboxRole::Inbox,
        "sent" | "sent messages" | "sent items" | "sent mail" => MailboxRole::Sent,
        "draft" | "drafts" => MailboxRole::Drafts,
        "trash" | "deleted messages" | "deleted items" | "bin" => MailboxRole::Trash,
        "junk" | "spam" => MailboxRole::Junk,
        "archive" => MailboxRole::Archive,
        "all mail" => MailboxRole::All,
        "starred" | "flagged" => MailboxRole::Flagged,
        "important" => MailboxRole::Important,
        _ => return None,
    })
}

/// Names an original without writing it: msgvault already stores it.
struct Referenced;

impl moonmail_core::BlobStore for Referenced {
    type Error = moonmail_store::BlobError;

    fn put(&self, bytes: &[u8]) -> Result<String, Self::Error> {
        Ok(moonmail_store::content_digest(bytes))
    }

    fn get(&self, _hash: &str) -> Result<Vec<u8>, Self::Error> {
        Err(moonmail_store::BlobError::ExternalMissing)
    }
}

/// Delete Moonmail's copies of originals msgvault keeps, after checking
/// msgvault returns the same bytes. Copies another account uses stay.
pub fn release(archive: &Archive, blobs: &FileBlobs) -> Result<usize> {
    let (mut after, mut released) = (String::new(), 0);
    loop {
        let hashes = archive.releasable_blobs(&after, 1000)?;
        let Some(last) = hashes.last() else {
            return Ok(released);
        };
        after = last.clone();
        for hash in &hashes {
            if blobs.release(hash)? {
                released += 1;
            }
        }
    }
}

struct Mirror<'a> {
    archive: &'a mut Archive,
    vault: &'a Connection,
    account: &'a MsgvaultAccount,
    roles: HashMap<String, Option<String>>,
    mailboxes: HashMap<String, i64>,
}

impl Mirror<'_> {
    fn new<'a>(
        archive: &'a mut Archive,
        vault: &'a Connection,
        account: &'a MsgvaultAccount,
    ) -> Result<Mirror<'a>> {
        let mut q = vault.prepare("SELECT name,system_role FROM labels WHERE source_id=?1")?;
        let roles = q
            .query_map([account.source], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Mirror {
            archive,
            vault,
            account,
            roles,
            mailboxes: HashMap::new(),
        })
    }

    /// Labels that are read state rather than places.
    fn place(label: &str) -> bool {
        label != "UNREAD"
    }

    fn mailboxes(&mut self, labels: &str) -> Result<Vec<i64>> {
        let names = if labels.is_empty() {
            vec!["All Mail"]
        } else {
            labels.split(SEPARATOR).collect()
        };
        names
            .into_iter()
            .map(|name| {
                if let Some(id) = self.mailboxes.get(name) {
                    return Ok(*id);
                }
                let system = self.roles.get(name).cloned().flatten();
                let id = self.archive.msgvault_mailbox(
                    &self.account.account,
                    name,
                    role(name, system.as_deref()),
                )?;
                self.mailboxes.insert(name.to_owned(), id);
                Ok(id)
            })
            .collect()
    }

    fn labels(&self, id: i64) -> Result<String> {
        let mut q = self.vault.prepare_cached(
            "SELECT l.name FROM message_labels ml JOIN labels l ON l.id=ml.label_id
             WHERE ml.message_id=?1 ORDER BY l.name",
        )?;
        let names = q
            .query_map([id], |r| r.get::<_, String>(0))?
            .filter(|name| name.as_ref().map_or(true, |name| Self::place(name)))
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(names.join(&SEPARATOR.to_string()))
    }

    /// Mirror newest first: new arrivals above the cursor, then older mail
    /// below the backfill mark one batch at a time, checking for arrivals
    /// between batches. Returns how many messages arrived.
    fn ingest(&mut self, progress: &mut impl FnMut(usize)) -> Result<usize> {
        let account = &self.account.account;
        let (mut cursor, mut backfill) = (self.account.cursor, self.account.backfill);
        if backfill < 0 {
            let newest: i64 = self.vault.query_row(
                "SELECT COALESCE(MAX(id),0) FROM messages WHERE source_id=?1 AND message_type='email'",
                [self.account.source],
                |r| r.get(0),
            )?;
            self.archive.begin_msgvault(account, newest)?;
            (cursor, backfill) = (newest, newest + 1);
        }
        let mut count = 0;
        loop {
            while let Some((last, arrived)) = self.batch(cursor, true)? {
                cursor = last;
                count += arrived;
                progress(count);
            }
            if backfill <= 0 {
                return Ok(count);
            }
            match self.batch(backfill, false)? {
                Some((last, arrived)) => {
                    backfill = last;
                    count += arrived;
                    progress(count);
                },
                None => {
                    self.archive.finish_msgvault_backfill(account)?;
                    backfill = 0;
                },
            }
        }
    }

    /// Mirror up to one batch of email above (`newer`) or below `mark`.
    /// Returns the last msgvault ID processed and how many were mirrored.
    fn batch(&mut self, mark: i64, newer: bool) -> Result<Option<(i64, usize)>> {
        let batch = {
            let mut q = self.vault.prepare_cached(if newer {
                "SELECT id,is_read,deleted_at IS NULL AND deleted_from_source_at IS NULL,
                 CAST(strftime('%s',COALESCE(internal_date,received_at,sent_at)) AS INTEGER)
                 FROM messages WHERE source_id=?1 AND id>?2 AND message_type='email'
                 ORDER BY id LIMIT ?3"
            } else {
                "SELECT id,is_read,deleted_at IS NULL AND deleted_from_source_at IS NULL,
                 CAST(strftime('%s',COALESCE(internal_date,received_at,sent_at)) AS INTEGER)
                 FROM messages WHERE source_id=?1 AND id<?2 AND message_type='email'
                 ORDER BY id DESC LIMIT ?3"
            })?;
            q.query_map(rusqlite::params![self.account.source, mark, BATCH], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<bool>>(1)?.unwrap_or(true),
                    r.get::<_, bool>(2)?,
                    r.get::<_, Option<i64>>(3)?.unwrap_or(0),
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let Some(last) = batch.last().map(|row| row.0) else {
            return Ok(None);
        };
        let parser = StalwartParser::default();
        let mut count = 0;
        for (id, seen, live, received_at) in batch {
            let raw = if live { self.raw(id)? } else { None };
            let Some(raw) = raw else {
                self.archive.skip_msgvault(&self.account.account, id)?;
                continue;
            };
            let labels = self.labels(id)?;
            let mailboxes = self.mailboxes(&labels)?;
            let origin = format!("msgvault:{id}");
            // msgvault keeps the original; Moonmail records where, not a copy.
            self.archive.reference_external(
                &moonmail_store::content_digest(&raw),
                &self.account.database,
                id,
            )?;
            let message = match self.archive.ingest(
                &parser,
                &Referenced,
                IncomingMessage {
                    account: &self.account.account,
                    mailbox: mailboxes[0],
                    origin: &origin,
                    epoch: 0,
                    uid: None,
                    seen,
                    starred: labels.split(SEPARATOR).any(|label| label == "STARRED"),
                    received_at,
                    raw: &raw,
                },
            ) {
                Ok(message) => message,
                Err(IngestError::Parser(_)) => {
                    tracing::warn!(id, "msgvault message has unreadable MIME; skipped");
                    self.archive.skip_msgvault(&self.account.account, id)?;
                    continue;
                },
                Err(error) => return Err(error.into()),
            };
            let recorded = self.archive.record_msgvault(
                &self.account.account,
                message,
                &MsgvaultChange {
                    vault: id,
                    seen,
                    labels: &labels,
                    mailboxes: Some(&mailboxes),
                },
            )?;
            if !recorded {
                return Ok(None);
            }
            count += 1;
        }
        Ok(Some((last, count)))
    }

    /// The original MIME, or `None` for a message msgvault holds no mail for.
    fn raw(&self, id: i64) -> Result<Option<Vec<u8>>> {
        let row = self
            .vault
            .prepare_cached(
                "SELECT raw_data,compression FROM message_raw
                 WHERE message_id=?1 AND raw_format='mime' AND COALESCE(encryption_version,0)=0",
            )?
            .query_row([id], |r| {
                Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, Option<String>>(1)?))
            })
            .optional()?;
        let Some((data, compression)) = row else {
            return Ok(None);
        };
        let bytes = match compression.as_deref() {
            None | Some("" | "none") => data,
            Some("zlib") => {
                let mut out = Vec::new();
                ZlibDecoder::new(data.as_slice())
                    .take(LIMIT + 1)
                    .read_to_end(&mut out)
                    .with_context(|| format!("Decompressing msgvault message {id}"))?;
                out
            },
            Some(other) => {
                tracing::warn!(id, other, "unsupported msgvault compression; skipped");
                return Ok(None);
            },
        };
        if bytes.len() as u64 > LIMIT {
            tracing::warn!(id, "msgvault message exceeds 64 MiB; skipped");
            return Ok(None);
        }
        Ok(Some(bytes))
    }

    /// Follow read state, label and deletion changes for mirrored messages.
    fn reconcile(&mut self) -> Result<usize> {
        let states = self.archive.msgvault_states(&self.account.account)?;
        if states.is_empty() {
            return Ok(0);
        }
        let current = {
            let mut q = self.vault.prepare_cached(
                "SELECT m.id,m.is_read,m.deleted_at IS NULL AND m.deleted_from_source_at IS NULL,
                 (SELECT group_concat(name,char(31)) FROM (SELECT l.name FROM message_labels ml
                  JOIN labels l ON l.id=ml.label_id WHERE ml.message_id=m.id AND l.name<>'UNREAD'
                  ORDER BY l.name))
                 FROM messages m WHERE m.source_id=?1 AND m.message_type='email' AND m.id BETWEEN ?3 AND ?2",
            )?;
            q.query_map(
                rusqlite::params![
                    self.account.source,
                    self.account.cursor,
                    self.account.backfill.max(0)
                ],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        (
                            r.get::<_, Option<bool>>(1)?.unwrap_or(true),
                            r.get::<_, bool>(2)?,
                            r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                        ),
                    ))
                },
            )?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?
        };
        let mut found = Vec::new();
        for (vault, old) in &states {
            match current.get(vault) {
                Some((seen, true, labels)) if *seen == old.seen && *labels == old.labels => {},
                Some((seen, true, labels)) => found.push((old, *vault, *seen, Some(labels))),
                _ => found.push((old, *vault, old.seen, None)),
            }
        }
        let mut mailboxes = Vec::with_capacity(found.len());
        for (_, _, _, labels) in &found {
            mailboxes.push(match labels {
                Some(labels) => Some(self.mailboxes(labels)?),
                None => None,
            });
        }
        let changes = found
            .iter()
            .zip(&mailboxes)
            .map(|((old, vault, seen, labels), mailboxes)| {
                (
                    *old,
                    MsgvaultChange {
                        vault: *vault,
                        seen: *seen,
                        labels: labels.map_or("", String::as_str),
                        mailboxes: mailboxes.as_deref(),
                    },
                )
            })
            .collect::<Vec<_>>();
        for chunk in changes.chunks(500) {
            self.archive.update_msgvault(&self.account.account, chunk)?;
        }
        Ok(changes.len())
    }
}

/// Mirror every msgvault account once. `progress` hears each batch.
pub fn sync(
    root: &Path,
    archive: &mut Archive,
    mut progress: impl FnMut(&str, usize),
) -> Result<usize> {
    let blobs = FileBlobs::new(root);
    let mut changed = 0;
    let mut errors = Vec::new();
    for account in archive.msgvault_accounts()? {
        let result = open(Path::new(&account.database)).and_then(|vault| {
            let mut mirror = Mirror::new(archive, &vault, &account)?;
            let fresh = mirror.reconcile()?;
            let arrived = mirror.ingest(&mut |count| progress(&account.account, count))?;
            Ok(fresh + arrived)
        });
        match result {
            Ok(count) => changed += count,
            Err(error) => errors.push(format!("{}: {error:#}", account.account)),
        }
    }
    let released = release(archive, &blobs)?;
    if released > 0 {
        tracing::info!(released, "released copies msgvault keeps");
    }
    anyhow::ensure!(errors.is_empty(), "{}", errors.join("\n"));
    Ok(changed)
}

/// The database and its write-ahead log change whenever msgvault commits.
fn stamp(database: &str) -> Vec<Option<(u64, std::time::SystemTime)>> {
    ["", "-wal"]
        .iter()
        .map(|suffix| {
            fs::metadata(format!("{database}{suffix}"))
                .and_then(|meta| Ok((meta.len(), meta.modified()?)))
                .ok()
        })
        .collect()
}

pub enum Event {
    Progress(String),
    Updated,
    Failed(String),
}

pub struct Worker {
    _stop: mpsc::Sender<()>,
    pub events: mpsc::Receiver<Event>,
}

impl Worker {
    pub fn start(root: PathBuf) -> Result<Self> {
        let (stop, signal) = mpsc::channel();
        let (send, events) = mpsc::channel();
        std::thread::Builder::new()
            .name("moonmail-msgvault".into())
            .spawn(move || {
                let mut seen = HashMap::new();
                let mut last_error = None;
                let mut archive = None;
                loop {
                    let report = scan(&root, &mut archive, &mut seen, &send);
                    let (changed, error) = match report {
                        Ok(changed) => (changed, None),
                        Err(error) => (false, Some(format!("{error:#}"))),
                    };
                    if changed || error != last_error {
                        if error.is_some() {
                            // Retry everything once the failure is fixed.
                            seen.clear();
                        }
                        last_error = error.clone();
                        let event = error.map_or(Event::Updated, Event::Failed);
                        if send.send(event).is_err() {
                            break;
                        }
                    }
                    if !matches!(
                        signal.recv_timeout(Duration::from_secs(5)),
                        Err(mpsc::RecvTimeoutError::Timeout)
                    ) {
                        break;
                    }
                }
            })?;
        Ok(Self {
            _stop: stop,
            events,
        })
    }
}

type Stamps = HashMap<String, Vec<Option<(u64, std::time::SystemTime)>>>;

fn scan(
    root: &Path,
    connection: &mut Option<Archive>,
    seen: &mut Stamps,
    send: &mpsc::Sender<Event>,
) -> Result<bool> {
    let lock = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("msgvault.lock"))?;
    match lock.try_lock() {
        Ok(()) => {},
        Err(fs::TryLockError::WouldBlock) => return Ok(false),
        Err(fs::TryLockError::Error(e)) => return Err(e.into()),
    }
    let archive = match connection {
        Some(archive) => archive,
        None => connection.insert(Archive::open(root)?),
    };
    let databases = archive
        .msgvault_accounts()?
        .into_iter()
        .map(|account| account.database)
        .collect::<Vec<_>>();
    let stamps = databases
        .iter()
        .map(|database| (database.clone(), stamp(database)))
        .collect::<Stamps>();
    if stamps == *seen {
        return Ok(false);
    }
    let changed = sync(root, archive, |account, count| {
        let _ = send.send(Event::Progress(format!(
            "{account}: {count} messages mirrored…"
        )));
    })?;
    *seen = stamps;
    Ok(changed > 0)
}

#[cfg(test)]
mod tests;
