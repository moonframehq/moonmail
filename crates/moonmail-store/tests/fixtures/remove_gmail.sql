ALTER TABLE mailboxes DROP COLUMN sync_enabled;
DROP INDEX gmail_messages;
ALTER TABLE messages DROP COLUMN gm_msgid;
ALTER TABLE messages DROP COLUMN gm_thrid;
ALTER TABLE messages DROP COLUMN gmail_labels;
ALTER TABLE keyword_operations RENAME TO new_keyword_operations;
CREATE TABLE keyword_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id), epoch INTEGER NOT NULL,
    uid INTEGER NOT NULL, keyword TEXT NOT NULL, value INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN ('queued','blocked')),
    detail TEXT NOT NULL DEFAULT '', UNIQUE(mailbox_id,epoch,uid,keyword)
);
INSERT INTO keyword_operations SELECT id,mailbox_id,epoch,uid,keyword,value,state,detail FROM new_keyword_operations WHERE gmail=0;
DROP TABLE new_keyword_operations;
