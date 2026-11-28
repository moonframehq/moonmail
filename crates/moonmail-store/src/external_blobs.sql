-- Originals kept by another local archive instead of a Moonmail file.
CREATE TABLE IF NOT EXISTS external_blobs (
    hash TEXT PRIMARY KEY, database TEXT NOT NULL, row INTEGER NOT NULL
);
INSERT OR IGNORE INTO external_blobs
    SELECT m.blob, a.database, v.vault FROM msgvault_messages v
    JOIN messages m ON m.id=v.message JOIN msgvault_accounts a ON a.account=v.account;
CREATE INDEX IF NOT EXISTS messages_blob ON messages(blob);
CREATE INDEX IF NOT EXISTS messages_rfc_id_any ON messages(rfc_id) WHERE rfc_id IS NOT NULL;
PRAGMA user_version=28;
