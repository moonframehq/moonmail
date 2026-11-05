use {
    crate::{FolderOutcome, IncomingMail, RemoteFolder, SyncError},
    moonmail_core::FolderChange,
    moonmail_store::{Archive, FolderOperation},
    std::collections::BTreeSet,
};

fn valid_namespace(operation: &FolderOperation, folders: &[RemoteFolder]) -> bool {
    match &operation.change {
        FolderChange::Create { name } => !folders.iter().any(|folder| folder.name == *name),
        FolderChange::Rename {
            source,
            destination,
        } => {
            let Some(root) = operation.members.first() else {
                return false;
            };
            let beneath = |name: &str, parent: &str| {
                name == parent
                    || root
                        .delimiter
                        .as_ref()
                        .is_some_and(|delimiter| name.starts_with(&format!("{parent}{delimiter}")))
            };
            let current: BTreeSet<_> = folders
                .iter()
                .filter(|folder| beneath(&folder.name, source))
                .map(|folder| folder.name.as_str())
                .collect();
            let expected = operation
                .members
                .iter()
                .map(|folder| folder.name.as_str())
                .collect();
            current == expected
                && !folders
                    .iter()
                    .any(|folder| beneath(&folder.name, destination))
                && operation.members.iter().all(|member| {
                    folders.iter().any(|folder| {
                        folder.name == member.name
                            && folder.delimiter == member.delimiter
                            && folder.selectable == member.selectable
                    })
                })
        },
        FolderChange::Delete { name } => {
            let Some(root) = operation.members.first() else {
                return false;
            };
            folders.iter().any(|folder| {
                folder.name == *name
                    && folder.delimiter == root.delimiter
                    && folder.selectable == root.selectable
            }) && !folders.iter().any(|folder| {
                root.delimiter
                    .as_ref()
                    .is_some_and(|delimiter| folder.name.starts_with(&format!("{name}{delimiter}")))
            })
        },
    }
}

pub(crate) fn apply<I: IncomingMail>(
    incoming: &mut I,
    account: &str,
    folders: &[RemoteFolder],
    archive: &mut Archive,
    progress: &mut impl FnMut(String),
    cancelled: &impl Fn() -> bool,
) -> Result<bool, SyncError<I::Error>> {
    let Some(operation) = archive.pending_folder_edit(account)? else {
        return Ok(false);
    };
    crate::sync::check_cancel(cancelled)?;
    if !valid_namespace(&operation, folders) {
        archive.fail_folder_edit(
            operation.id,
            "Folder names or hierarchy changed since queuing; no command was sent",
        )?;
        return Ok(false);
    }
    for member in &operation.members {
        crate::sync::check_cancel(cancelled)?;
        if member.selectable
            && incoming
                .folder_epoch(&member.name)
                .map_err(SyncError::Incoming)?
                != member.epoch
        {
            archive.fail_folder_edit(
                operation.id,
                "Source UIDVALIDITY changed since queuing; no command was sent",
            )?;
            return Ok(false);
        }
    }
    progress(match &operation.change {
        FolderChange::Create { name } => format!("Creating folder {name}"),
        FolderChange::Rename {
            source,
            destination,
        } => format!("Renaming {source} to {destination}"),
        FolderChange::Delete { name } => format!("Deleting folder {name}"),
    });
    crate::sync::check_cancel(cancelled)?;
    if !archive.claim_folder_edit(operation.id)? {
        return Ok(false);
    }
    match incoming.edit_folder(&operation.change) {
        Ok(FolderOutcome::Applied) => {
            archive.finish_folder_edit(
                &operation,
                "done",
                "Server acknowledged folder change; folder refresh follows",
            )?;
            Ok(true)
        },
        Ok(FolderOutcome::Unsupported) => {
            archive.finish_folder_edit(
                &operation,
                "failed",
                "Adapter does not support folder changes; no command was sent",
            )?;
            Ok(false)
        },
        Err(error) => {
            archive.finish_folder_edit(&operation,"unknown","Folder command failed or its response was lost. Sync and inspect names and contents before dismissing; no automatic retry.")?;
            Err(SyncError::Incoming(error))
        },
    }
}
