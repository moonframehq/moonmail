CREATE TABLE search_sources (
    search_id INTEGER NOT NULL REFERENCES saved_searches(id) ON DELETE CASCADE,
    source_id INTEGER NOT NULL REFERENCES saved_searches(id) ON DELETE RESTRICT,
    PRIMARY KEY(search_id,source_id), CHECK(search_id<>source_id)
);
CREATE INDEX search_source_users ON search_sources(source_id);
PRAGMA user_version=13;
