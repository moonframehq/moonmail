ALTER TABLE mailboxes ADD COLUMN sync_enabled INTEGER NOT NULL DEFAULT 1 CHECK(sync_enabled IN (0,1));
ALTER TABLE messages ADD COLUMN gm_msgid BLOB;
ALTER TABLE messages ADD COLUMN gm_thrid BLOB;
CREATE UNIQUE INDEX gmail_messages ON messages(account,gm_msgid) WHERE gm_msgid IS NOT NULL;
ALTER TABLE messages ADD COLUMN gmail_labels TEXT;
CREATE TABLE new_keyword_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id), epoch INTEGER NOT NULL,
    uid INTEGER NOT NULL, keyword TEXT NOT NULL, value INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN ('queued','blocked')),
    detail TEXT NOT NULL DEFAULT '', gmail INTEGER NOT NULL DEFAULT 0,
    UNIQUE(mailbox_id,epoch,uid,gmail,keyword)
);
INSERT INTO new_keyword_operations(id,mailbox_id,epoch,uid,keyword,value,state,detail)
    SELECT id,mailbox_id,epoch,uid,keyword,value,state,detail FROM keyword_operations;
UPDATE sqlite_sequence SET seq=MAX(seq,COALESCE((SELECT seq FROM sqlite_sequence WHERE name='keyword_operations'),0))
    WHERE name='new_keyword_operations';
DROP TABLE keyword_operations;
ALTER TABLE new_keyword_operations RENAME TO keyword_operations;
PRAGMA user_version=21;
