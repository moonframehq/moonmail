CREATE TABLE thread_nodes (
    id INTEGER PRIMARY KEY, account TEXT NOT NULL, kind TEXT NOT NULL,
    value TEXT NOT NULL, component INTEGER NOT NULL DEFAULT 0,
    UNIQUE(account,kind,value)
);
CREATE INDEX thread_components ON thread_nodes(component,id);
CREATE TABLE thread_members (
    message_id INTEGER PRIMARY KEY REFERENCES messages(id),
    node_id INTEGER NOT NULL UNIQUE REFERENCES thread_nodes(id),
    limited INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE thread_edges (
    source INTEGER NOT NULL REFERENCES messages(id),
    a INTEGER NOT NULL REFERENCES thread_nodes(id),
    b INTEGER NOT NULL REFERENCES thread_nodes(id),
    kind TEXT NOT NULL, PRIMARY KEY(source,a,b,kind), CHECK(a<b)
);
CREATE INDEX thread_edges_a ON thread_edges(a,b);
CREATE INDEX thread_edges_b ON thread_edges(b,a);
CREATE TABLE thread_pending (message_id INTEGER PRIMARY KEY REFERENCES messages(id));
INSERT INTO thread_pending SELECT id FROM messages;
PRAGMA user_version=14;
