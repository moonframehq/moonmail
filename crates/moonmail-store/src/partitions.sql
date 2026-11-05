CREATE TABLE search_partitions (
    search_id INTEGER PRIMARY KEY REFERENCES saved_searches(id) ON DELETE CASCADE,
    field TEXT NOT NULL CHECK(field IN ('sender','recipient','keyword'))
);
PRAGMA user_version=19;
