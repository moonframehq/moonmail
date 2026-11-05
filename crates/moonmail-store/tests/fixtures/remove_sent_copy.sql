DROP INDEX outbox_sent_copy_pending;
ALTER TABLE outbox DROP COLUMN sent_copy_state;
ALTER TABLE outbox DROP COLUMN sent_copy_detail;
ALTER TABLE outbox DROP COLUMN sent_copy_mailbox;
