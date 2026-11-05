CREATE TABLE saved_searches (
    id INTEGER PRIMARY KEY, name TEXT NOT NULL UNIQUE, source TEXT NOT NULL,
    predicate TEXT NOT NULL
);
PRAGMA user_version=12;
