CREATE TABLE IF NOT EXISTS jmap_accounts (
    account TEXT PRIMARY KEY, remote TEXT NOT NULL, mail_state TEXT, folder_state TEXT
);
CREATE TABLE IF NOT EXISTS jmap_folders (
    mailbox INTEGER PRIMARY KEY REFERENCES mailboxes(id), account TEXT NOT NULL, remote TEXT NOT NULL,
    UNIQUE(account,remote)
);
-- Stable local handles for the durable edit journals. Epoch zero is reserved
-- for JMAP; IMAP UIDVALIDITY values are strictly positive.
CREATE TABLE IF NOT EXISTS jmap_emails (
    uid INTEGER PRIMARY KEY AUTOINCREMENT CHECK(uid BETWEEN 1 AND 4294967295),
    account TEXT NOT NULL, remote TEXT NOT NULL, UNIQUE(account,remote)
);
PRAGMA user_version=21;
