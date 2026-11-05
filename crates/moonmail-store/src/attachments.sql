ALTER TABLE drafts ADD COLUMN attachments_json TEXT NOT NULL DEFAULT '[]';
PRAGMA user_version=7;
