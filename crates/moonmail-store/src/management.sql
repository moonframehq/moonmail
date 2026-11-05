ALTER TABLE mailboxes ADD COLUMN server_role TEXT;
ALTER TABLE mailboxes ADD COLUMN user_role TEXT;
UPDATE mailboxes SET server_role='inbox' WHERE name='INBOX' COLLATE NOCASE;
CREATE UNIQUE INDEX mailbox_user_role ON mailboxes(account,user_role) WHERE user_role IS NOT NULL;
CREATE TABLE move_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    message_id INTEGER NOT NULL REFERENCES messages(id),
    source_id INTEGER NOT NULL REFERENCES mailboxes(id),
    destination_id INTEGER NOT NULL REFERENCES mailboxes(id),
    epoch INTEGER NOT NULL, uid INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','running','done','failed','unknown','cancelled','dismissed')),
    detail TEXT NOT NULL DEFAULT '', created INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE UNIQUE INDEX move_active_source ON move_operations(source_id,epoch,uid)
    WHERE state IN ('queued','running','unknown');
CREATE INDEX move_queue ON move_operations(state,id);
CREATE INDEX move_active_message ON move_operations(message_id)
    WHERE state IN ('queued','running','unknown');
PRAGMA user_version=5;
