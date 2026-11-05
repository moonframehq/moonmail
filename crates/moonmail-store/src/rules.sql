CREATE TABLE keyword_rules (
    id INTEGER PRIMARY KEY AUTOINCREMENT, revision INTEGER NOT NULL DEFAULT 1,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id), name TEXT NOT NULL,
    source TEXT NOT NULL, predicate TEXT NOT NULL, keyword TEXT NOT NULL,
    value INTEGER NOT NULL, enabled INTEGER NOT NULL, UNIQUE(mailbox_id,name)
);
CREATE INDEX keyword_rules_mailbox ON keyword_rules(mailbox_id,enabled,id);
CREATE TABLE keyword_rule_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    rule_id INTEGER NOT NULL, revision INTEGER NOT NULL, name TEXT NOT NULL,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id),
    message_id INTEGER NOT NULL REFERENCES messages(id), epoch INTEGER NOT NULL, uid INTEGER,
    keyword TEXT NOT NULL, value INTEGER NOT NULL, mode TEXT NOT NULL,
    state TEXT NOT NULL, detail TEXT NOT NULL
);
PRAGMA user_version=18;
