ALTER TABLE messages ADD COLUMN header_date INTEGER;
ALTER TABLE messages ADD COLUMN received_at INTEGER;
PRAGMA user_version=11;
