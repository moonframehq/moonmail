CREATE TABLE occurrence_keywords (
    mailbox_id INTEGER NOT NULL, message_id INTEGER NOT NULL, keyword TEXT NOT NULL,
    PRIMARY KEY(mailbox_id,message_id,keyword),
    FOREIGN KEY(mailbox_id,message_id) REFERENCES occurrences(mailbox_id,message_id) ON DELETE CASCADE
);
CREATE INDEX keyword_messages ON occurrence_keywords(keyword,message_id);
CREATE TABLE keyword_labels (
    account TEXT NOT NULL, keyword TEXT NOT NULL, label TEXT NOT NULL,
    PRIMARY KEY(account,keyword), UNIQUE(account,label)
);
CREATE TABLE keyword_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id), epoch INTEGER NOT NULL,
    uid INTEGER NOT NULL, keyword TEXT NOT NULL, value INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN ('queued','blocked')),
    detail TEXT NOT NULL DEFAULT '',
    UNIQUE(mailbox_id,epoch,uid,keyword)
);
PRAGMA user_version=17;
