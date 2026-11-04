CREATE TABLE search_documents (
    id INTEGER PRIMARY KEY CHECK (id >= 0),
    subject TEXT NOT NULL,
    body TEXT NOT NULL,
    from_addr TEXT NOT NULL,
    to_addr TEXT NOT NULL,
    cc_addr TEXT NOT NULL
);

CREATE VIRTUAL TABLE documents_fts USING fts5(
    subject, body, from_addr, to_addr, cc_addr,
    content='search_documents', content_rowid='id',
    tokenize='unicode61 remove_diacritics 1'
);

CREATE TRIGGER documents_insert AFTER INSERT ON search_documents BEGIN
    INSERT INTO documents_fts(rowid, subject, body, from_addr, to_addr, cc_addr)
    VALUES (new.id, new.subject, new.body, new.from_addr, new.to_addr, new.cc_addr);
END;

CREATE TRIGGER documents_delete AFTER DELETE ON search_documents BEGIN
    INSERT INTO documents_fts(documents_fts, rowid, subject, body, from_addr, to_addr, cc_addr)
    VALUES ('delete', old.id, old.subject, old.body, old.from_addr, old.to_addr, old.cc_addr);
END;

CREATE TRIGGER documents_update AFTER UPDATE ON search_documents BEGIN
    INSERT INTO documents_fts(documents_fts, rowid, subject, body, from_addr, to_addr, cc_addr)
    VALUES ('delete', old.id, old.subject, old.body, old.from_addr, old.to_addr, old.cc_addr);
    INSERT INTO documents_fts(rowid, subject, body, from_addr, to_addr, cc_addr)
    VALUES (new.id, new.subject, new.body, new.from_addr, new.to_addr, new.cc_addr);
END;

PRAGMA user_version = 1;
