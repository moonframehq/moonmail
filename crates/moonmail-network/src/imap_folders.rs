use {
    crate::Result,
    moonmail_core::FolderChange,
    std::io::{Read, Write},
};

pub(crate) fn edit<T: Read + Write>(
    session: &mut imap::Session<T>,
    change: &FolderChange,
) -> Result<()> {
    // EXAMINE another mailbox deselects the source without expunging it. INBOX
    // is never an allowed source in the frozen namespace-operation contract.
    match change {
        FolderChange::Create { name } => session.create(name)?,
        FolderChange::Rename {
            source,
            destination,
        } => {
            session.examine("INBOX")?;
            session.rename(source, destination)?;
        },
        FolderChange::Delete { name } => {
            session.examine("INBOX")?;
            session.delete(name)?;
        },
    }
    Ok(())
}
