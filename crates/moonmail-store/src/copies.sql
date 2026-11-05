ALTER TABLE move_operations ADD COLUMN kind TEXT NOT NULL DEFAULT 'move' CHECK(kind IN ('move','copy'));
PRAGMA user_version=16;
