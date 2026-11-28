CREATE TABLE IF NOT EXISTS msgvault_dismissed (
    database TEXT NOT NULL, source INTEGER NOT NULL, PRIMARY KEY (database, source)
);
PRAGMA user_version=27;
