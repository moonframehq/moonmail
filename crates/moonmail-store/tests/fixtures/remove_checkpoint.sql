DROP TRIGGER invalidate_discarded_keyword;
DROP TRIGGER invalidate_imap_checkpoint;
ALTER TABLE mailboxes DROP COLUMN highest_modseq;
ALTER TABLE mailboxes DROP COLUMN inventory_upper;
ALTER TABLE mailboxes DROP COLUMN inventory_generation;
