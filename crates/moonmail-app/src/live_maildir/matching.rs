use super::{Entry, HashMap, HashSet, MaildirFile};

pub(super) struct Matches {
    pub pairs: Vec<Option<usize>>,
    pub used: HashSet<usize>,
    pub candidates: Vec<Option<usize>>,
}

pub(super) fn match_entries(
    entries: &[Entry],
    old: &[MaildirFile],
    folders: &HashMap<String, i64>,
) -> Matches {
    let mut pairs = vec![None; entries.len()];
    let mut used = HashSet::new();
    pair(
        &mut pairs,
        &mut used,
        old,
        |f| {
            (
                f.mailbox,
                f.location.stem.as_str(),
                f.location.fingerprint.as_str(),
            )
        },
        |i| {
            (
                folders[&entries[i].folder],
                entries[i].stem.as_str(),
                entries[i].fingerprint.as_str(),
            )
        },
    );
    pair(
        &mut pairs,
        &mut used,
        old,
        |f| (f.location.stem.as_str(), f.location.fingerprint.as_str()),
        |i| (entries[i].stem.as_str(), entries[i].fingerprint.as_str()),
    );
    pair(
        &mut pairs,
        &mut used,
        old,
        |f| f.location.fingerprint.as_str(),
        |i| entries[i].fingerprint.as_str(),
    );
    // Some clients move by copying then unlinking. Consider only unambiguous
    // stems, and verify immutable bytes before transferring identity or edits.
    let mut candidates = pairs.clone();
    let mut excluded = used.clone();
    pair(
        &mut candidates,
        &mut excluded,
        old,
        |f| f.location.stem.as_str(),
        |i| entries[i].stem.as_str(),
    );
    Matches {
        pairs,
        used,
        candidates,
    }
}

// Match exact directory entries before matching renames. Hard-linked copies
// have independent rows. An ambiguous group never inherits another row's edits.
fn pair<'a, K: std::hash::Hash + Eq>(
    pairs: &mut [Option<usize>],
    used: &mut HashSet<usize>,
    old: &'a [MaildirFile],
    old_key: impl Fn(&'a MaildirFile) -> K,
    new_key: impl Fn(usize) -> K,
) {
    if used.len() == old.len() || used.len() == pairs.len() {
        return;
    }
    let mut available = HashMap::new();
    for (i, file) in old.iter().enumerate().filter(|(i, _)| !used.contains(i)) {
        available
            .entry(old_key(file))
            .and_modify(|v| *v = None)
            .or_insert(Some(i));
    }
    let mut unique = HashMap::new();
    for (i, _) in pairs.iter().enumerate().filter(|(_, v)| v.is_none()) {
        unique
            .entry(new_key(i))
            .and_modify(|v| *v = None)
            .or_insert(Some(i));
    }
    for (key, entry) in unique {
        if let (Some(i), Some(Some(j))) = (entry, available.get(&key)) {
            pairs[i] = Some(*j);
            used.insert(*j);
        }
    }
}
