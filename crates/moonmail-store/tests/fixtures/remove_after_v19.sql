DROP TRIGGER IF EXISTS jmap_deletion_arrival;
DROP TRIGGER invalidate_discarded_keyword;
DROP TRIGGER invalidate_imap_checkpoint;
ALTER TABLE mailboxes DROP COLUMN highest_modseq;
ALTER TABLE mailboxes DROP COLUMN inventory_upper;
ALTER TABLE mailboxes DROP COLUMN inventory_generation;
DROP TRIGGER draft_cleanup;
DROP TRIGGER draft_account_change;
DROP TABLE draft_uploads;
DROP TABLE expunge_operations;
DROP TABLE deletion_inventory;
DROP INDEX draft_remote_identity;
DROP INDEX draft_upload_queue;
ALTER TABLE drafts DROP COLUMN remote_mailbox;
ALTER TABLE drafts DROP COLUMN remote_epoch;
ALTER TABLE drafts DROP COLUMN remote_uid;
ALTER TABLE drafts DROP COLUMN remote_synced_at;
ALTER TABLE drafts DROP COLUMN pending_remote_delete;
ALTER TABLE drafts DROP COLUMN dirty;
ALTER TABLE mailboxes DROP COLUMN sync_enabled;
DROP INDEX gmail_messages;
ALTER TABLE messages DROP COLUMN gm_msgid;
ALTER TABLE messages DROP COLUMN gm_thrid;
ALTER TABLE messages DROP COLUMN gmail_labels;
ALTER TABLE keyword_operations RENAME TO new_keyword_operations;
CREATE TABLE keyword_operations (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mailbox_id INTEGER NOT NULL REFERENCES mailboxes(id), epoch INTEGER NOT NULL,
    uid INTEGER NOT NULL, keyword TEXT NOT NULL, value INTEGER NOT NULL,
    state TEXT NOT NULL DEFAULT 'queued' CHECK(state IN ('queued','blocked')),
    detail TEXT NOT NULL DEFAULT '', UNIQUE(mailbox_id,epoch,uid,keyword)
);
INSERT INTO keyword_operations SELECT id,mailbox_id,epoch,uid,keyword,value,state,detail FROM new_keyword_operations WHERE gmail=0;
DROP TABLE new_keyword_operations;
DROP INDEX outbox_sent_copy_pending;
ALTER TABLE outbox DROP COLUMN sent_copy_state;
ALTER TABLE outbox DROP COLUMN sent_copy_detail;
ALTER TABLE outbox DROP COLUMN sent_copy_mailbox;
