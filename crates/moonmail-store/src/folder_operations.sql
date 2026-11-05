CREATE TABLE folder_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT, account TEXT NOT NULL,
    change_json TEXT NOT NULL, members_json TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('queued','running','done','failed','unknown','cancelled','dismissed')),
    detail TEXT NOT NULL DEFAULT '', created INTEGER NOT NULL DEFAULT (unixepoch())
);
CREATE UNIQUE INDEX folder_active_account ON folder_operations(account)
    WHERE state IN ('queued','running','unknown');
CREATE INDEX folder_queue ON folder_operations(state,id);
PRAGMA user_version=15;
