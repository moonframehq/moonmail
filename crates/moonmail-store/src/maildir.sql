CREATE TABLE IF NOT EXISTS maildir_accounts (
    account TEXT PRIMARY KEY, root TEXT NOT NULL UNIQUE, device TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS maildir_folders (
    mailbox INTEGER PRIMARY KEY REFERENCES mailboxes(id), relative TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS maildir_files (
    message INTEGER PRIMARY KEY REFERENCES messages(id),
    mailbox INTEGER NOT NULL REFERENCES mailboxes(id), stem TEXT NOT NULL, fingerprint TEXT NOT NULL,
    want_seen INTEGER, want_star INTEGER
);
CREATE INDEX IF NOT EXISTS maildir_mailbox ON maildir_files(mailbox);
PRAGMA user_version=25;
