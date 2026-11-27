DROP TRIGGER draft_cleanup;
DROP TRIGGER draft_account_change;
CREATE TABLE new_expunge_operations (
    id INTEGER PRIMARY KEY, account TEXT NOT NULL, mailbox TEXT NOT NULL,
    epoch INTEGER NOT NULL CHECK(epoch>=0), uid INTEGER NOT NULL CHECK(uid>0),
    draft_id TEXT, state TEXT NOT NULL DEFAULT 'queued',
    remote_folder TEXT, confirmed_role TEXT,
    CHECK(epoch>0 OR (remote_folder IS NOT NULL AND confirmed_role IN ('trash','junk') AND confirmed_role IS NOT NULL))
);
INSERT INTO new_expunge_operations(id,account,mailbox,epoch,uid,draft_id,state)
    SELECT id,account,mailbox,epoch,uid,draft_id,state FROM expunge_operations;
DROP TABLE expunge_operations;
ALTER TABLE new_expunge_operations RENAME TO expunge_operations;
CREATE UNIQUE INDEX expunge_imap_identity ON expunge_operations(account,mailbox,epoch,uid) WHERE epoch>0;
CREATE UNIQUE INDEX expunge_jmap_identity ON expunge_operations(account,remote_folder,uid) WHERE epoch=0;
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

-- A JMAP Email keeps its ID when moved. An old email entering Trash also
-- invalidates an open empty-folder confirmation, regardless of its local UID.
CREATE TABLE IF NOT EXISTS jmap_deletion_revisions(mailbox INTEGER PRIMARY KEY, revision INTEGER NOT NULL);
CREATE TRIGGER IF NOT EXISTS jmap_deletion_arrival AFTER INSERT ON occurrences WHEN NEW.epoch=0
    AND EXISTS(SELECT 1 FROM jmap_folders WHERE mailbox=NEW.mailbox_id)
BEGIN
    INSERT INTO jmap_deletion_revisions VALUES(NEW.mailbox_id,1)
    ON CONFLICT(mailbox) DO UPDATE SET revision=revision+1;
END;
PRAGMA user_version=27;
