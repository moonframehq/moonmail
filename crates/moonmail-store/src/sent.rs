use {
    crate::{Archive, Error, Result},
    moonmail_core::MailboxRole,
    rusqlite::params,
};

impl Archive {
    /// Reconcile against durably ingested occurrences in the configured Sent
    /// folder. This observes a matching Message-ID, not delivery to recipients.
    /// Neither SMTP outcome nor retry eligibility is changed. Multiple matches
    /// are explicitly ambiguous. Missing data never schedules SMTP or APPEND.
    pub fn reconcile_sent(&mut self, account: &str) -> Result<()> {
        let mailbox = match self.role_mailbox(account, MailboxRole::Sent) {
            Ok(id) => Some(id),
            Err(Error::Invalid(_)) => None,
            Err(error) => return Err(error),
        };
        let transaction = self.connection.transaction()?;
        if let Some(mailbox) = mailbox {
            transaction.execute(
                "UPDATE outbox SET
                 sent_status=CASE (SELECT COUNT(*) FROM messages m JOIN occurrences o ON o.message_id=m.id
                    WHERE m.account=outbox.account AND m.rfc_id=outbox.rfc_id AND o.mailbox_id=?2 AND o.uid IS NOT NULL)
                    WHEN 0 THEN 'missing' WHEN 1 THEN 'visible' ELSE 'ambiguous' END,
                 sent_message=(SELECT CASE WHEN COUNT(*)=1 THEN MIN(m.id) END FROM messages m JOIN occurrences o ON o.message_id=m.id
                    WHERE m.account=outbox.account AND m.rfc_id=outbox.rfc_id AND o.mailbox_id=?2 AND o.uid IS NOT NULL)
                 WHERE account=?1 AND state IN ('sent','unknown') AND rfc_id IS NOT NULL",
                params![account,mailbox],
            )?;
        } else {
            transaction.execute("UPDATE outbox SET sent_status='unavailable',sent_message=NULL WHERE account=?1 AND state IN ('sent','unknown') AND rfc_id IS NOT NULL", [account])?;
        }
        // Finding a unique remote copy resolves an uncertain APPEND without
        // attempting either SMTP or IMAP delivery again.
        transaction.execute("UPDATE outbox SET sent_copy_state='saved',sent_copy_detail='Observed a matching remote Sent copy' WHERE account=?1 AND sent_status='visible' AND sent_copy_state IN ('pending','unknown')", [account])?;
        transaction.commit()?;
        Ok(())
    }
}
