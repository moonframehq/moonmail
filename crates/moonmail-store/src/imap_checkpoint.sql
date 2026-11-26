-- Eight network-order bytes preserve the entire unsigned IMAP MODSEQ domain.
ALTER TABLE mailboxes ADD COLUMN highest_modseq BLOB CHECK(length(highest_modseq)=8);
ALTER TABLE mailboxes ADD COLUMN inventory_upper INTEGER;
ALTER TABLE mailboxes ADD COLUMN inventory_generation INTEGER NOT NULL DEFAULT 0;
CREATE TRIGGER invalidate_imap_checkpoint AFTER UPDATE OF epoch,active ON mailboxes
WHEN NEW.epoch<>OLD.epoch OR NEW.active<>OLD.active
BEGIN
    UPDATE mailboxes SET highest_modseq=NULL, inventory_upper=NULL,
        inventory_generation=inventory_generation+1 WHERE id=NEW.id;
END;
-- A blocked keyword overlays the last observed flags locally. Discarding it
-- must restore server state even when the mailbox's MODSEQ has not changed.
CREATE TRIGGER invalidate_discarded_keyword AFTER DELETE ON keyword_operations
WHEN OLD.state='blocked'
BEGIN
    UPDATE mailboxes SET highest_modseq=NULL, inventory_upper=NULL,
        inventory_generation=inventory_generation+1 WHERE id=OLD.mailbox_id;
END;
PRAGMA user_version=23;
