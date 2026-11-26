//! Polling coalesces deliveries and rename bursts on one connection-owning worker.
use {
    anyhow::{Context, Result, ensure},
    inventory::{Entry, Inventory, fingerprint, identity},
    moonmail_mime::StalwartParser,
    moonmail_store::{
        Archive, FileBlobs, IncomingMessage, MaildirAccount, MaildirFile, MaildirLocation,
    },
    std::{
        collections::{HashMap, HashSet},
        fs,
        path::Path,
    },
};
mod delivery;
mod inventory;
mod matching;
mod worker;
use delivery::{read_delivery, write_flags};
pub use worker::Worker;

pub fn attach(root: &Path, path: &Path) -> Result<()> {
    let path = path.canonicalize()?;
    let archive = root.canonicalize()?;
    ensure!(
        !path.starts_with(&archive) && !archive.starts_with(&path),
        "Choose a Maildir outside the archive"
    );
    let device = identity(&fs::metadata(&path)?);
    ensure!(
        !Inventory::read(&path, &device)?.folders.is_empty(),
        "No complete Maildir found"
    );
    Archive::open(root)?.register_maildir(
        path.to_str().context("Maildir path must be UTF-8")?,
        &device,
    )?;
    Ok(())
}

fn reconcile(archive: &mut Archive, blobs: &FileBlobs, account: &MaildirAccount) -> Result<bool> {
    let scan = Inventory::read(Path::new(&account.root), &account.device)?;
    let mut folders = HashMap::new();
    let previous = archive.maildir_folders(&account.account)?;
    let mut removed = Vec::new();
    for (name, (id, relative)) in &previous {
        if !scan.folders.contains_key(name) {
            // A removed mailbox disappears as a directory. If the path is
            // still there but its Maildir structure is gone, retry intact.
            inventory::confirm_removed(&account.root, relative)?;
            removed.push(*id);
        }
    }
    let mut changed = !removed.is_empty();
    for (name, relative) in &scan.folders {
        let id = match previous.get(name) {
            Some((id, old)) if old == relative => *id,
            _ => {
                changed = true;
                archive.ensure_maildir_folder(&account.account, name, relative)?
            },
        };
        folders.insert(name.clone(), id);
    }
    let old = archive.maildir_files(&account.account)?;
    let entries = &scan.entries;
    let matching::Matches {
        pairs,
        mut used,
        candidates,
    } = matching::match_entries(entries, &old, &folders);
    let mut observations = Vec::new();
    let parser = StalwartParser::default();
    for (i, (entry, matched)) in entries.iter().zip(pairs).enumerate() {
        let mut delivery = None;
        let mut matched = matched;
        if matched.is_none() {
            let (raw, timestamp) = read_delivery(entry)?;
            if let Some(j) = candidates[i]
                && moonmail_store::content_digest(&raw) == old[j].blob
            {
                used.insert(j);
                matched = Some(j);
            } else {
                delivery = Some((raw, timestamp));
            }
        }
        let mailbox = folders[&entry.folder];
        let seen = entry.flags.contains('S');
        let star = entry.flags.contains('F');
        if let Some(j) = matched {
            let file = &old[j];
            let desired = (
                file.want_seen.unwrap_or(seen),
                file.want_star.unwrap_or(star),
            );
            if file.mailbox == mailbox
                && file.location.stem == entry.stem
                && file.location.fingerprint == entry.fingerprint
                && file.seen == seen
                && file.starred == star
                && file.want_seen.is_none()
                && file.want_star.is_none()
            {
                continue;
            }
            observations.push((file, entry, mailbox, desired, desired != (seen, star)));
        } else if let Some((raw, received_at)) = delivery {
            archive.ingest_maildir(
                &parser,
                blobs,
                IncomingMessage {
                    account: &account.account,
                    mailbox,
                    origin: &uuid::Uuid::new_v4().to_string(),
                    epoch: 0,
                    uid: None,
                    seen,
                    starred: star,
                    received_at,
                    raw: &raw,
                },
                &MaildirLocation {
                    stem: entry.stem.clone(),
                    fingerprint: entry.fingerprint.clone(),
                },
            )?;
        }
        changed = true;
    }
    let missing = old
        .iter()
        .enumerate()
        .filter_map(|(i, f)| (!used.contains(&i)).then_some(f))
        .collect::<Vec<_>>();
    if !missing.is_empty() || !removed.is_empty() {
        scan.validate()?;
        archive.prune_maildir(&missing, &removed)?;
        changed = true;
    }
    for (file, entry, mailbox, desired, rename) in observations {
        if rename {
            write_flags(entry, desired)?;
        } else {
            ensure!(
                fingerprint(&fs::symlink_metadata(&entry.path)?) == entry.fingerprint,
                "Maildir entry changed before observation"
            );
        }
        archive.observe_maildir(
            file,
            mailbox,
            &MaildirLocation {
                stem: entry.stem.clone(),
                fingerprint: entry.fingerprint.clone(),
            },
            desired.0,
            desired.1,
        )?;
    }
    Ok(changed)
}

#[cfg(test)]
mod tests;
