-- What an outside client (the agent CLI, MCP) asks the open app to do: show
-- something, read what its window shows, or act as a click in it would.
CREATE TABLE IF NOT EXISTS agent_requests (
    id INTEGER PRIMARY KEY, at INTEGER NOT NULL, source TEXT NOT NULL, kind TEXT NOT NULL,
    target TEXT NOT NULL, summary TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending', detail TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS agent_requests_pending ON agent_requests(state, id);
PRAGMA user_version=28;
