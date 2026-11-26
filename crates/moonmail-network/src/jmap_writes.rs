use {
    crate::jmap::{Client, JmapError, Result, field},
    serde_json::{Value, json},
};
pub(crate) fn pointer(id: &str) -> String {
    id.replace('~', "~0").replace('/', "~1")
}
fn check(reply: &Value, field: &str, id: &str) -> Result<()> {
    let ok = if field == "destroyed" {
        reply[field]
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == id))
    } else {
        reply[field].get(id).is_some()
    };
    if ok {
        Ok(())
    } else if reply["notUpdated"][id]["type"] == "notFound"
        || reply["notDestroyed"][id]["type"] == "notFound"
    {
        Err(JmapError::NotFound)
    } else if ["notUpdated", "notCreated", "notDestroyed"]
        .iter()
        .any(|f| reply[*f].get(id).is_some())
    {
        Err(JmapError::Rejected)
    } else {
        Err(JmapError::Protocol)
    }
}
impl Client {
    pub fn keyword(&self, id: &str, keyword: &str, enabled: bool) -> Result<()> {
        self.patch(
            "Email/set",
            id,
            json!({format!("keywords/{}",pointer(keyword)):enabled.then_some(true)}),
        )
    }
    pub fn transfer(&self, id: &str, source: Option<&str>, destination: &str) -> Result<()> {
        let mut patch = json!({format!("mailboxIds/{}",pointer(destination)):true});
        if let Some(source) = source.filter(|s| *s != destination) {
            patch[format!("mailboxIds/{}", pointer(source))] = Value::Null;
        }
        self.patch("Email/set", id, patch)
    }
    fn patch(&self, method: &str, id: &str, patch: Value) -> Result<()> {
        check(
            &self.method(method, json!({"update":{id:patch}}))?,
            "updated",
            id,
        )
    }
    pub fn destroy_email(&self, id: &str) -> Result<()> {
        check(
            &self.method("Email/set", json!({"destroy":[id]}))?,
            "destroyed",
            id,
        )
    }
    pub fn create_mailbox(&self, name: &str, parent: Option<&str>) -> Result<String> {
        let result = self.method(
            "Mailbox/set",
            json!({"create":{"new":{"name":name,"parentId":parent}}}),
        )?;
        check(&result, "created", "new")?;
        Ok(field(&result["created"]["new"], "id")?.into())
    }
    pub fn rename_mailbox(&self, id: &str, name: &str, parent: Option<&str>) -> Result<()> {
        self.patch("Mailbox/set", id, json!({"name":name,"parentId":parent}))
    }
    pub fn delete_mailbox(&self, id: &str) -> Result<()> {
        check(
            &self.method("Mailbox/set", json!({"destroy":[id]}))?,
            "destroyed",
            id,
        )
    }
}

impl Client {
    fn flush_flags(
        &self,
        account: &str,
        archive: &mut moonmail_store::Archive,
        boundary: &impl Fn() -> crate::Result<()>,
    ) -> crate::Result<()> {
        let mut after = 0;
        loop {
            let flags = archive.jmap_flags(account, after, self.set_batch)?;
            let Some(last) = flags.last() else {
                return Ok(());
            };
            after = last.id;
            boundary()?;
            let mut updates = serde_json::Map::new();
            for flag in &flags {
                let patch = updates.entry(&flag.remote).or_insert_with(|| json!({}));
                patch[format!("keywords/${}", flag.kind)] = json!(flag.value.then_some(true));
            }
            let reply = self.method("Email/set", json!({"update":updates}))?;
            let mut accepted = Vec::new();
            let mut failure = None;
            for flag in &flags {
                match check(&reply, "updated", &flag.remote) {
                    Ok(()) | Err(JmapError::NotFound) => accepted.push(flag),
                    Err(error) => {
                        failure.get_or_insert(error);
                    },
                }
            }
            archive.acknowledge_jmap_flags(&accepted)?;
            if let Some(error) = failure {
                return Err(error.into());
            }
        }
    }

    pub(crate) fn flush_edits(
        &self,
        account: &str,
        archive: &mut moonmail_store::Archive,
        stop: &impl Fn() -> bool,
    ) -> crate::Result<()> {
        let boundary = || {
            if stop() {
                Err(crate::Error::Cancelled)
            } else {
                Ok(())
            }
        };
        self.flush_flags(account, archive, &boundary)?;
        let mut after = 0;
        loop {
            let ops = archive.keyword_operations(account, after)?;
            if ops.is_empty() {
                break;
            }
            for op in ops {
                after = op.id;
                if op.state != "queued" || op.epoch != 0 {
                    continue;
                }
                boundary()?;
                if !archive.keyword_source_exists(op.id)? {
                    archive.block_keyword(op.id, "Source membership disappeared")?;
                    continue;
                }
                let result = self.keyword(
                    &archive.jmap_remote(account, op.uid)?,
                    &op.keyword,
                    op.value,
                );
                match result {
                    Ok(()) => archive.acknowledge_keyword(op.id)?,
                    Err(JmapError::Rejected | JmapError::NotFound) => {
                        archive.block_keyword(op.id, "Fastmail rejected the keyword change")?
                    },
                    Err(e) => return Err(e.into()),
                }
            }
        }
        let folders = archive.jmap_folder_map(account)?;
        for op in archive.pending_moves(account)? {
            boundary()?;
            if op.epoch != 0 {
                continue;
            }
            let find = |id| {
                folders
                    .iter()
                    .find(|(_, local, _)| *local == id)
                    .map(|(remote, _, _)| remote)
            };
            let (Some(source), Some(dest)) = (find(op.source_id), find(op.destination_id)) else {
                archive.fail_queued_move(op.id, "Mailbox disappeared")?;
                continue;
            };
            let email = archive.jmap_remote(account, op.uid)?;
            let data = self.method(
                "Email/get",
                json!({"ids":[email],"properties":["mailboxIds"]}),
            )?;
            if data["list"][0]["mailboxIds"][source] != true {
                archive.fail_queued_move(op.id, "Source membership disappeared")?;
                continue;
            }
            let mut patch = json!({format!("mailboxIds/{}",pointer(dest)):true});
            if op.kind == "move" {
                patch[format!("mailboxIds/{}", pointer(source))] = Value::Null;
            }
            let args = json!({"ifInState":field(&data,"state")?,"update":{&email:patch}});
            if archive.claim_move(op.id)? {
                let outcome = self
                    .method("Email/set", args)
                    .and_then(|reply| check(&reply, "updated", &email));
                let (state, detail) = operation_result(&outcome);
                archive.finish_move(op.id, state, detail)?;
                outcome?;
            }
        }
        if let Some(op) = archive.pending_folder_edit(account)? {
            boundary()?;
            let (state, _) = archive.jmap_states(account)?;
            let by_path = |path: &str| {
                folders
                    .iter()
                    .find(|(_, _, p)| p == path)
                    .map(|(remote, _, _)| remote.as_str())
            };
            let parts = |path: &str| -> Result<Value> {
                let (parent, name) = match path.rsplit_once('/') {
                    Some((prefix, name)) => {
                        (Some(by_path(prefix).ok_or(JmapError::Rejected)?), name)
                    },
                    None => (None, path),
                };
                Ok(json!({"name":name.replace("%2F","/").replace("%25","%"),"parentId":parent}))
            };
            if op.members.iter().any(|m| {
                m.epoch != 0 || !folders.iter().any(|(_, id, p)| *id == m.id && *p == m.name)
            }) {
                archive.fail_folder_edit(op.id, "Folder changed since queuing")?;
                return Ok(());
            }
            use moonmail_core::FolderChange;
            let (mut args, key, id) = match &op.change {
                FolderChange::Create { name } => {
                    (json!({"create":{"new":parts(name)?}}), "created", "new")
                },
                FolderChange::Rename {
                    source,
                    destination,
                } => {
                    let id = by_path(source).ok_or(JmapError::Rejected)?;
                    (json!({"update":{id:parts(destination)?}}), "updated", id)
                },
                FolderChange::Delete { name } => {
                    let id = by_path(name).ok_or(JmapError::Rejected)?;
                    (
                        json!({"destroy":[id],"onDestroyRemoveEmails":true}),
                        "destroyed",
                        id,
                    )
                },
            };
            args["ifInState"] = json!(state);
            if archive.claim_folder_edit(op.id)? {
                let outcome = self
                    .method("Mailbox/set", args)
                    .and_then(|reply| check(&reply, key, id));
                let (state, detail) = operation_result(&outcome);
                archive.finish_folder_edit(&op, state, detail)?;
                outcome?;
            }
        }
        Ok(())
    }
}
fn operation_result(result: &Result<()>) -> (&'static str, &'static str) {
    match result {
        Ok(()) => ("done", "Fastmail acknowledged the change"),
        Err(JmapError::Rejected | JmapError::NotFound | JmapError::Unauthorized) => {
            ("failed", "Fastmail rejected the change")
        },
        Err(_) => (
            "unknown",
            "JMAP response was interrupted; inspect the folder before retrying",
        ),
    }
}
