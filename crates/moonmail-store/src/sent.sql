ALTER TABLE outbox ADD COLUMN rfc_id TEXT;
ALTER TABLE outbox ADD COLUMN sent_status TEXT NOT NULL DEFAULT 'untracked';
ALTER TABLE outbox ADD COLUMN sent_message INTEGER REFERENCES messages(id);
CREATE INDEX outbox_sent_tracking ON outbox(account,state,rfc_id);
PRAGMA user_version=9;
