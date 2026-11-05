ALTER TABLE outbox ADD COLUMN sent_copy_state TEXT NOT NULL DEFAULT 'server';
ALTER TABLE outbox ADD COLUMN sent_copy_detail TEXT NOT NULL DEFAULT '';
ALTER TABLE outbox ADD COLUMN sent_copy_mailbox TEXT NOT NULL DEFAULT '';
CREATE INDEX outbox_sent_copy_pending ON outbox(account,sent_copy_state,id);
PRAGMA user_version=20;
