CREATE TABLE mailboxes (
    id INTEGER PRIMARY KEY, account TEXT NOT NULL, name TEXT NOT NULL,
    epoch INTEGER NOT NULL DEFAULT 0, UNIQUE(account, name)
);
CREATE TABLE messages (
    id INTEGER PRIMARY KEY, account TEXT NOT NULL, origin TEXT NOT NULL,
    blob TEXT NOT NULL, subject TEXT NOT NULL, sender TEXT NOT NULL,
    address TEXT NOT NULL, recipients TEXT NOT NULL, date INTEGER NOT NULL,
    snippet TEXT NOT NULL, rfc_id TEXT, thread_key TEXT NOT NULL,
    UNIQUE(account, origin)
);
CREATE INDEX messages_date ON messages(date DESC, id DESC);
CREATE INDEX messages_thread ON messages(account, thread_key, date DESC);
CREATE INDEX messages_rfc_id ON messages(account, rfc_id);
CREATE TABLE bodies (message_id INTEGER PRIMARY KEY REFERENCES messages(id), text TEXT NOT NULL, html TEXT);
CREATE TABLE addresses (
    message_id INTEGER NOT NULL REFERENCES messages(id), role TEXT NOT NULL,
    address TEXT NOT NULL, PRIMARY KEY(message_id, role, address)
);
CREATE INDEX address_lookup ON addresses(address, role, message_id);
CREATE TABLE occurrences (
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id), message_id INTEGER NOT NULL REFERENCES messages(id),
    epoch INTEGER NOT NULL, uid INTEGER, seen INTEGER NOT NULL, flagged INTEGER NOT NULL,
    PRIMARY KEY(mailbox_id, message_id), UNIQUE(mailbox_id, epoch, uid)
);
CREATE INDEX occurrences_message ON occurrences(message_id, seen, flagged);
CREATE INDEX occurrences_unread ON occurrences(mailbox_id, message_id) WHERE seen=0;
CREATE TABLE changes (seq INTEGER PRIMARY KEY AUTOINCREMENT, message_id INTEGER NOT NULL);
CREATE TABLE projection (name TEXT PRIMARY KEY, cursor INTEGER NOT NULL);
INSERT INTO projection VALUES ('search', 0);
CREATE TABLE flag_operations (
    id INTEGER PRIMARY KEY, mailbox_id INTEGER NOT NULL, epoch INTEGER NOT NULL,
    uid INTEGER NOT NULL, kind TEXT NOT NULL, value INTEGER NOT NULL,
    UNIQUE(mailbox_id, epoch, uid, kind)
);
CREATE TABLE drafts (
    id TEXT PRIMARY KEY, recipients TEXT NOT NULL, subject TEXT NOT NULL,
    body TEXT NOT NULL, reply_id TEXT NOT NULL DEFAULT '', updated INTEGER NOT NULL
);
CREATE TABLE outbox (
    id INTEGER PRIMARY KEY, draft_id TEXT NOT NULL UNIQUE, account TEXT NOT NULL,
    blob TEXT NOT NULL, sender TEXT NOT NULL, recipients TEXT NOT NULL,
    subject TEXT NOT NULL, state TEXT NOT NULL, detail TEXT NOT NULL DEFAULT '',
    created INTEGER NOT NULL
);
PRAGMA user_version=1;
