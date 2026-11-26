DROP TRIGGER invalidate_discarded_keyword;
DROP TRIGGER invalidate_imap_checkpoint;
ALTER TABLE mailboxes DROP COLUMN highest_modseq;
ALTER TABLE mailboxes DROP COLUMN inventory_upper;
ALTER TABLE mailboxes DROP COLUMN inventory_generation;
DROP INDEX outbox_sent_copy_pending;
ALTER TABLE outbox DROP COLUMN sent_copy_state;
ALTER TABLE outbox DROP COLUMN sent_copy_detail;
ALTER TABLE outbox DROP COLUMN sent_copy_mailbox;
