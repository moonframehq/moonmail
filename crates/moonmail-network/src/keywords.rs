use {
    crate::{IncomingMail, KeywordOutcome, SyncError},
    moonmail_store::Archive,
};

pub(crate) fn apply<I: IncomingMail>(
    incoming: &mut I,
    account: &str,
    archive: &Archive,
    progress: &mut impl FnMut(String),
    cancelled: &impl Fn() -> bool,
) -> Result<(), SyncError<I::Error>> {
    let mut after = 0;
    loop {
        crate::sync::check_cancel(cancelled)?;
        let operations = archive.keyword_operations(account, after)?;
        if operations.is_empty() {
            return Ok(());
        }
        for op in operations {
            after = op.id;
            if op.state != "queued" {
                continue;
            }
            if !archive.keyword_source_exists(op.id)? {
                archive.block_keyword(op.id, "Source UID/UIDVALIDITY no longer exists")?;
                continue;
            }
            crate::sync::check_cancel(cancelled)?;
            match incoming
                .keyword(&op.mailbox, op.epoch, op.uid, &op.keyword, op.value)
                .map_err(SyncError::Incoming)?
            {
                KeywordOutcome::Applied => archive.acknowledge_keyword(op.id)?,
                KeywordOutcome::Unsupported => {
                    archive
                        .block_keyword(op.id, "Server does not permit this permanent keyword")?;
                    progress(format!(
                        "Keyword {} is unsupported in {}; review Tags",
                        op.keyword, op.mailbox
                    ));
                },
                KeywordOutcome::Stale => {
                    archive.block_keyword(op.id, "Source UID/UIDVALIDITY no longer exists")?
                },
            }
        }
    }
}
