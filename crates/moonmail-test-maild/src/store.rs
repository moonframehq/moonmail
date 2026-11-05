use {
    crate::{Error, Result},
    std::collections::{BTreeMap, BTreeSet},
};

pub const ACCOUNTS: [&str; 2] = ["alice@moonmail.test", "bob@moonmail.test"];
pub const PASSWORD: &str = "moonmail-test-only";
pub const MAX_MESSAGE: usize = 1024 * 1024;
const MAX_BYTES: usize = 32 * 1024 * 1024;
const MAX_MESSAGES: usize = 256;

pub(crate) fn validate_flags(flags: &BTreeSet<String>) -> Result {
    if flags.len() > 64 || flags.iter().any(|flag| flag.len() > 64) {
        return Err(Error::Invalid("Fixture flag limit reached"));
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct Message {
    pub uid: u32,
    pub raw: Vec<u8>,
    pub flags: BTreeSet<String>,
}

pub(crate) struct Folder {
    pub next: u32,
    pub messages: Vec<Message>,
}

pub(crate) struct Store {
    pub epoch: u32,
    pub accounts: BTreeMap<String, BTreeMap<String, Folder>>,
    pub server_sent: bool,
    bytes: usize,
}

impl Store {
    pub fn new(server_sent: bool) -> Self {
        let mut accounts = BTreeMap::new();
        for account in ACCOUNTS {
            accounts.insert(
                account.into(),
                ["INBOX", "Sent", "Archive", "Trash", "Junk", "Drafts"]
                    .into_iter()
                    .map(|name| {
                        (
                            name.into(),
                            Folder {
                                next: 1,
                                messages: Vec::new(),
                            },
                        )
                    })
                    .collect(),
            );
        }
        let bytes = *uuid::Uuid::new_v4().as_bytes();
        Self {
            epoch: u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]).max(1),
            accounts,
            server_sent,
            bytes: 0,
        }
    }

    pub fn folder(&self, account: &str, folder: &str) -> Result<&Folder> {
        self.accounts
            .get(account)
            .and_then(|a| a.get(folder))
            .ok_or(Error::Invalid("Unknown mailbox"))
    }

    pub fn folder_mut(&mut self, account: &str, folder: &str) -> Result<&mut Folder> {
        self.accounts
            .get_mut(account)
            .and_then(|a| a.get_mut(folder))
            .ok_or(Error::Invalid("Unknown mailbox"))
    }

    pub fn append(
        &mut self,
        account: &str,
        folder: &str,
        raw: &[u8],
        flags: BTreeSet<String>,
    ) -> Result<u32> {
        validate_flags(&flags)?;
        if raw.len() > MAX_MESSAGE || self.bytes + raw.len() > MAX_BYTES {
            return Err(Error::Invalid("Fixture storage limit reached"));
        }
        let mailbox = self.folder_mut(account, folder)?;
        if mailbox.messages.len() >= MAX_MESSAGES {
            return Err(Error::Invalid("Fixture mailbox message limit reached"));
        }
        let uid = mailbox.next;
        mailbox.next = uid
            .checked_add(1)
            .ok_or(Error::Invalid("UID limit reached"))?;
        mailbox.messages.push(Message {
            uid,
            raw: raw.to_vec(),
            flags,
        });
        self.bytes += raw.len();
        Ok(uid)
    }

    pub fn deliver(&mut self, sender: &str, recipients: &[String], raw: &[u8]) -> Result {
        let recipients: BTreeSet<_> = recipients.iter().collect();
        if recipients.is_empty()
            || !self.accounts.contains_key(sender)
            || recipients
                .iter()
                .any(|r| !self.accounts.contains_key(r.as_str()))
        {
            return Err(Error::Invalid("Only fixture accounts can send or receive"));
        }
        let copies = recipients.len() + usize::from(self.server_sent);
        if raw.len() > MAX_MESSAGE || self.bytes + raw.len() * copies > MAX_BYTES {
            return Err(Error::Invalid("Fixture storage limit reached"));
        }
        // Preflight all targets before publishing any delivery.
        for recipient in &recipients {
            self.preflight(recipient, "INBOX")?;
        }
        if self.server_sent {
            self.preflight(sender, "Sent")?;
        }
        for recipient in recipients {
            self.append(recipient, "INBOX", raw, BTreeSet::new())?;
        }
        if self.server_sent {
            self.append(sender, "Sent", raw, BTreeSet::from(["\\Seen".into()]))?;
        }
        Ok(())
    }

    pub fn remove(&mut self, account: &str, folder: &str, index: usize) -> Result {
        let folder = self.folder_mut(account, folder)?;
        if index >= folder.messages.len() {
            return Err(Error::Invalid("Unknown message"));
        }
        let message = folder.messages.remove(index);
        self.bytes -= message.raw.len();
        Ok(())
    }

    fn preflight(&self, account: &str, folder: &str) -> Result {
        let folder = self.folder(account, folder)?;
        if folder.next == u32::MAX || folder.messages.len() >= MAX_MESSAGES {
            return Err(Error::Invalid("Fixture mailbox limit reached"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_preflights_all_targets_and_sent_before_publishing() -> Result {
        let mut store = Store::new(true);
        store.folder_mut(ACCOUNTS[1], "INBOX")?.next = u32::MAX;
        assert!(
            store
                .deliver(ACCOUNTS[0], &ACCOUNTS.map(str::to_owned), b"message")
                .is_err()
        );
        assert!(store.folder(ACCOUNTS[0], "INBOX")?.messages.is_empty());
        assert_eq!(store.folder(ACCOUNTS[0], "INBOX")?.next, 1);
        store.folder_mut(ACCOUNTS[1], "INBOX")?.next = 1;
        store.folder_mut(ACCOUNTS[0], "Sent")?.next = u32::MAX;
        assert!(
            store
                .deliver(ACCOUNTS[0], &[ACCOUNTS[1].into()], b"message")
                .is_err()
        );
        assert!(store.folder(ACCOUNTS[1], "INBOX")?.messages.is_empty());
        assert_eq!(store.bytes, 0);
        Ok(())
    }

    #[test]
    fn quota_failure_is_atomic_and_expunge_reclaims_bytes_without_reusing_uids() -> Result {
        let mut store = Store::new(false);
        let raw = vec![b'x'; MAX_MESSAGE];
        for _ in 0..32 {
            store.append(ACCOUNTS[0], "INBOX", &raw, BTreeSet::new())?;
        }
        assert!(
            store
                .deliver(ACCOUNTS[0], &ACCOUNTS.map(str::to_owned), b"new")
                .is_err()
        );
        assert_eq!(store.folder(ACCOUNTS[0], "INBOX")?.next, 33);
        assert!(store.folder(ACCOUNTS[1], "INBOX")?.messages.is_empty());
        store.remove(ACCOUNTS[0], "INBOX", 0)?;
        assert_eq!(
            store.append(ACCOUNTS[0], "INBOX", &raw, BTreeSet::new())?,
            33
        );
        assert_eq!(store.bytes, MAX_BYTES);
        Ok(())
    }

    #[test]
    fn empty_messages_are_bounded_and_duplicate_recipients_deliver_once() -> Result {
        let mut store = Store::new(false);
        store.deliver(
            ACCOUNTS[0],
            &[ACCOUNTS[1].into(), ACCOUNTS[1].into()],
            b"one",
        )?;
        assert_eq!(store.folder(ACCOUNTS[1], "INBOX")?.messages.len(), 1);
        for _ in 1..MAX_MESSAGES {
            store.append(ACCOUNTS[1], "INBOX", b"", BTreeSet::new())?;
        }
        assert!(
            store
                .append(ACCOUNTS[1], "INBOX", b"", BTreeSet::new())
                .is_err()
        );
        Ok(())
    }
}
