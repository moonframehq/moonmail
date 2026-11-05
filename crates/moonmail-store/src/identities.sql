CREATE TABLE identities (
    id TEXT PRIMARY KEY, account TEXT NOT NULL, name TEXT NOT NULL,
    address TEXT NOT NULL, reply_to TEXT NOT NULL, signature TEXT NOT NULL,
    is_default INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX identities_default ON identities(account) WHERE is_default=1;
ALTER TABLE drafts ADD COLUMN identity_json TEXT;
PRAGMA user_version=10;
