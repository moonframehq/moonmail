CREATE TABLE IF NOT EXISTS msgvault_accounts (
    account TEXT PRIMARY KEY, database TEXT NOT NULL, source INTEGER NOT NULL,
    cursor INTEGER NOT NULL DEFAULT 0, UNIQUE(database, source)
);
CREATE TABLE IF NOT EXISTS msgvault_messages (
    message INTEGER PRIMARY KEY REFERENCES messages(id), account TEXT NOT NULL,
    vault INTEGER NOT NULL, seen INTEGER NOT NULL, labels TEXT NOT NULL, UNIQUE(account, vault)
);
PRAGMA user_version=26;
