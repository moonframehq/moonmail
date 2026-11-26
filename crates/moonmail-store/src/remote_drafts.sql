ALTER TABLE drafts ADD COLUMN remote_mailbox TEXT;
ALTER TABLE drafts ADD COLUMN remote_epoch INTEGER;
ALTER TABLE drafts ADD COLUMN remote_uid INTEGER;
ALTER TABLE drafts ADD COLUMN remote_synced_at INTEGER;
ALTER TABLE drafts ADD COLUMN pending_remote_delete INTEGER NOT NULL DEFAULT 0;
ALTER TABLE drafts ADD COLUMN dirty INTEGER NOT NULL DEFAULT 1;
CREATE UNIQUE INDEX draft_remote_identity ON drafts(account,remote_mailbox,remote_epoch,remote_uid);
CREATE INDEX draft_upload_queue ON drafts(account,dirty,updated);
-- An upload survives the composer row, including discard or queue-for-send during APPEND.
CREATE TABLE draft_uploads (
    draft_id TEXT PRIMARY KEY, account TEXT NOT NULL, mailbox TEXT NOT NULL,
    subject TEXT NOT NULL, revision INTEGER NOT NULL, message_id TEXT NOT NULL UNIQUE, state TEXT NOT NULL DEFAULT 'unknown' CHECK(state IN ('unknown','unsupported'))
);
CREATE TABLE expunge_operations (
    id INTEGER PRIMARY KEY, account TEXT NOT NULL, mailbox TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch>0), uid INTEGER NOT NULL CHECK(uid>0),
    draft_id TEXT, state TEXT NOT NULL DEFAULT 'queued',
    UNIQUE(account,mailbox,epoch,uid)
);
CREATE INDEX expunge_queue ON expunge_operations(account,state,id);
CREATE TRIGGER draft_cleanup BEFORE DELETE ON drafts WHEN OLD.remote_uid IS NOT NULL BEGIN
    INSERT OR IGNORE INTO expunge_operations(account,mailbox,epoch,uid,draft_id)
    VALUES(OLD.account,OLD.remote_mailbox,OLD.remote_epoch,OLD.remote_uid,OLD.id);
END;
CREATE TRIGGER draft_account_change AFTER UPDATE OF account ON drafts
WHEN NEW.account<>OLD.account AND OLD.remote_uid IS NOT NULL BEGIN
    INSERT OR IGNORE INTO expunge_operations(account,mailbox,epoch,uid,draft_id)
    VALUES(OLD.account,OLD.remote_mailbox,OLD.remote_epoch,OLD.remote_uid,OLD.id);
    UPDATE drafts SET remote_mailbox=NULL,remote_epoch=NULL,remote_uid=NULL,remote_synced_at=NULL,pending_remote_delete=1 WHERE id=NEW.id;
END;
CREATE TABLE deletion_inventory(mailbox_id INTEGER NOT NULL,epoch INTEGER NOT NULL,uid INTEGER NOT NULL,
    PRIMARY KEY(mailbox_id,epoch,uid));
PRAGMA user_version=21;
