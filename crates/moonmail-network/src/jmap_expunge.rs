use {
    crate::{
        Error, Result,
        jmap::{Client, JmapError, field},
        jmap_writes::pointer,
    },
    moonmail_store::Archive,
    serde_json::json,
    std::collections::{BTreeSet, HashMap, HashSet},
};

impl Client {
    pub(crate) fn flush_deletions(
        &self,
        account: &str,
        archive: &mut Archive,
        stop: &impl Fn() -> bool,
    ) -> Result<()> {
        loop {
            if stop() {
                return Err(Error::Cancelled);
            }
            let page = archive.jmap_deletions(account, self.set_batch.min(self.batch as u16))?;
            let Some(first) = page.first() else {
                return Ok(());
            };
            let ids: Vec<_> = page.iter().map(|op| &op.email).collect();
            let mut replies = self.calls(&[
                json!(["Mailbox/get",{"accountId":self.account,"ids":[first.folder],"properties":["id","role"]},"folder"]),
                json!(["Email/get",{"accountId":self.account,"ids":ids,"properties":["id","mailboxIds"]},"mail"]),
            ],false)?.into_iter();
            let folders = replies.next().ok_or(JmapError::Protocol)?;
            let mail = replies.next().ok_or(JmapError::Protocol)?;
            let state = field(&mail, "state")?;
            let folder = folders["list"]
                .as_array()
                .ok_or(JmapError::Protocol)?
                .iter()
                .find(|f| f["id"] == first.folder);
            let mut outstanding: HashSet<_> = ids.iter().map(|id| id.as_str()).collect();
            let mut records = HashMap::new();
            for email in mail["list"].as_array().ok_or(JmapError::Protocol)? {
                let id = field(email, "id")?;
                let members = email["mailboxIds"].as_object().ok_or(JmapError::Protocol)?;
                if !outstanding.remove(id)
                    || members.is_empty()
                    || members.values().any(|v| v != true)
                {
                    return Err(JmapError::Protocol.into());
                }
                records.insert(id, members);
            }
            for missing in mail["notFound"].as_array().ok_or(JmapError::Protocol)? {
                if !outstanding.remove(missing.as_str().ok_or(JmapError::Protocol)?) {
                    return Err(JmapError::Protocol.into());
                }
            }
            if !outstanding.is_empty() {
                return Err(JmapError::Protocol.into());
            }
            let role = moonmail_core::MailboxRole::parse(&first.role).ok_or(JmapError::Protocol)?;
            let mapped = match archive.role_mailbox(account, role) {
                Ok(id) => Some(id),
                Err(moonmail_store::Error::Invalid(_)) => None,
                Err(e) => return Err(e.into()),
            };
            let valid = mapped.is_some()
                && mapped == first.mailbox
                && folder.is_some_and(|f| first.user_role || f["role"] == first.role);
            if stop() {
                return Err(Error::Cancelled);
            }
            let claims = archive.claim_jmap_deletions(&page)?;
            let mut destroy = BTreeSet::new();
            let mut update = serde_json::Map::new();
            let mut claimed = Vec::new();
            for (op, won) in page.iter().zip(claims) {
                if !won {
                    continue;
                }
                let target = records.get(op.email.as_str());
                let missing = target.is_none();
                let present = target.is_some_and(|members| members.contains_key(&op.folder));
                if !valid || !present {
                    archive
                        .finish_expunge(op.id, if valid && missing { "done" } else { "stale" })?;
                } else if target.is_some_and(|folders| folders.len() == 1) {
                    destroy.insert(&op.email);
                    claimed.push((op, "destroyed", "notDestroyed"));
                } else {
                    update.insert(
                        op.email.clone(),
                        json!({format!("mailboxIds/{}",pointer(&op.folder)):null}),
                    );
                    claimed.push((op, "updated", "notUpdated"));
                }
            }
            if claimed.is_empty() {
                continue;
            }
            // Claim before the network write. A lost response stays unknown until the user retries.
            let result = self.method(
                "Email/set",
                json!({"ifInState":state,"destroy":destroy,"update":update}),
            );
            let reply = match result {
                Ok(reply) => reply,
                Err(JmapError::StateMismatch) => {
                    for (op, _, _) in claimed {
                        archive.finish_expunge(op.id, "stale")?;
                    }
                    continue;
                },
                Err(error) => return Err(error.into()),
            };
            for (op, success, failed) in claimed {
                let reason = reply[failed][&op.email]["type"].as_str();
                let acknowledged = if success == "destroyed" {
                    reply[success]
                        .as_array()
                        .is_some_and(|v| v.iter().any(|id| id == &op.email))
                } else {
                    reply[success].get(&op.email).is_some()
                };
                if acknowledged || reason == Some("notFound") {
                    archive.finish_expunge(op.id, "done")?;
                } else if reason.is_some() {
                    archive.finish_expunge(op.id, "unsupported")?;
                } else {
                    return Err(JmapError::Protocol.into());
                }
            }
        }
    }
}
