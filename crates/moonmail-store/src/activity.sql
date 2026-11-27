-- Every agent tool call made through the agent CLI or MCP, reads included,
-- newest last. The app shows new writes and refreshes; nothing here is
-- authoritative.
CREATE TABLE IF NOT EXISTS activity (
    id INTEGER PRIMARY KEY, at INTEGER NOT NULL, source TEXT NOT NULL,
    tool TEXT NOT NULL DEFAULT '', writes INTEGER NOT NULL DEFAULT 1,
    ok INTEGER NOT NULL DEFAULT 1, summary TEXT NOT NULL
);
PRAGMA user_version=27;
