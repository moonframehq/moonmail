ALTER TABLE outbox ADD COLUMN draft_json TEXT;
ALTER TABLE outbox ADD COLUMN restored_draft_id TEXT;
PRAGMA user_version=8;
