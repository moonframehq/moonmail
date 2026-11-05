use {
    moonmail_core::{Mailbox, MailboxRole},
    serde::Serialize,
};

#[derive(Serialize)]
pub struct Folder {
    key: String,
    name: String,
    id: Option<i64>,
    unread: u64,
    icon: Option<&'static str>,
    children: Vec<Folder>,
}

/// Infer missing structural parents using each server's LIST delimiter. An
/// account is always a separate root; a NIL delimiter leaves names intact.
pub fn folders(mailboxes: &[Mailbox]) -> Vec<Folder> {
    let mut roots: Vec<Folder> = Vec::new();
    for mailbox in mailboxes {
        let index = roots
            .iter()
            .position(|node| node.name == mailbox.account)
            .unwrap_or_else(|| {
                roots.push(Folder {
                    key: format!("account:{}", mailbox.account),
                    name: mailbox.account.clone(),
                    id: None,
                    unread: 0,
                    icon: None,
                    children: Vec::new(),
                });
                roots.len() - 1
            });
        let root = &mut roots[index];
        root.unread += mailbox.unread;
        let parts: Vec<_> = match mailbox.delimiter.as_deref().filter(|s| !s.is_empty()) {
            Some(delimiter) => mailbox.name.split(delimiter).collect(),
            None => vec![mailbox.name.as_str()],
        };
        insert(&mut root.children, &root.key, &parts, mailbox);
    }
    fn sort(nodes: &mut [Folder]) {
        nodes.sort_by_key(|node| node.name.to_lowercase());
        for node in nodes {
            sort(&mut node.children);
        }
    }
    sort(&mut roots);
    roots
}

fn insert(nodes: &mut Vec<Folder>, parent: &str, parts: &[&str], mailbox: &Mailbox) {
    let Some((name, remaining)) = parts.split_first() else {
        return;
    };
    let index = nodes
        .iter()
        .position(|node| node.name == *name)
        .unwrap_or_else(|| {
            nodes.push(Folder {
                key: format!("{parent}/{}:{name}", name.len()),
                name: (*name).to_owned(),
                id: None,
                unread: 0,
                icon: None,
                children: Vec::new(),
            });
            nodes.len() - 1
        });
    let node = &mut nodes[index];
    node.unread += mailbox.unread;
    if remaining.is_empty() {
        node.id = mailbox.selectable.then_some(mailbox.id);
        node.icon = mailbox.role.map(|role| match role {
            MailboxRole::Inbox => "inbox",
            MailboxRole::Archive => "archive",
            MailboxRole::Sent => "send",
            MailboxRole::Drafts => "file-text",
            MailboxRole::Junk => "ban",
            MailboxRole::Trash => "trash-2",
            MailboxRole::All => "mails",
            MailboxRole::Flagged => "flag",
        });
    } else {
        insert(&mut node.children, &node.key, remaining, mailbox);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn respects_server_delimiters_flat_names_and_nonselectable_parents() {
        let mailbox = |id, name: &str, delimiter: Option<&str>, selectable| Mailbox {
            epoch: 0,
            id,
            account: "test@example.org".into(),
            name: name.into(),
            delimiter: delimiter.map(str::to_owned),
            selectable,
            role: None,
            total: 1,
            unread: 1,
        };
        let tree = folders(&[
            mailbox(1, "Work.Project", Some("."), true),
            mailbox(2, "Work", Some("."), false),
            mailbox(3, "Literal/Slash", None, true),
        ]);
        assert_eq!(tree[0].children.len(), 2);
        assert_eq!(tree[0].children[0].name, "Literal/Slash");
        let work = &tree[0].children[1];
        assert_eq!(work.id, None);
        assert_eq!(work.children[0].id, Some(1));
    }
}
