ALTER TABLE messages ADD COLUMN reply_addresses_json TEXT;
ALTER TABLE drafts ADD COLUMN cc TEXT NOT NULL DEFAULT '';
ALTER TABLE drafts ADD COLUMN bcc TEXT NOT NULL DEFAULT '';
ALTER TABLE drafts ADD COLUMN account TEXT NOT NULL DEFAULT '';
UPDATE drafts SET updated=updated*1000;
CREATE INDEX drafts_updated ON drafts(updated DESC,id DESC);
PRAGMA user_version=6;
