mod content;

use {
    crate::{
        Error, Result,
        jmap::{Client, JmapError, field},
    },
    moonmail_core::MailboxRole,
    moonmail_store::{Archive, FileBlobs, JmapFolder},
    serde::Deserialize,
    serde_json::{Value, json},
    std::collections::{BTreeSet, HashMap},
};

const PROPERTIES: &[&str] = &[
    "id",
    "blobId",
    "mailboxIds",
    "keywords",
    "size",
    "receivedAt",
];
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Folder {
    id: String,
    name: String,
    parent_id: Option<String>,
    role: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Email {
    id: String,
    blob_id: String,
    mailbox_ids: HashMap<String, bool>,
    keywords: HashMap<String, bool>,
    size: u64,
    received_at: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Changes {
    updated_properties: Option<Vec<String>>,
    old_state: String,
    new_state: String,
    has_more_changes: bool,
    created: Vec<String>,
    updated: Vec<String>,
    destroyed: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Emails {
    list: Vec<Email>,
    not_found: Vec<String>,
}
fn cancel(stop: &impl Fn() -> bool) -> Result<()> {
    if stop() {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T> {
    Ok(serde_json::from_value(value).map_err(JmapError::Json)?)
}

impl Client {
    fn refresh_folders(&self, archive: &mut Archive, account: &str) -> Result<()> {
        let result = self.method(
            "Mailbox/get",
            json!({"properties":["id","parentId","name","role"]}),
        )?;
        let folders: Vec<Folder> = decode(result["list"].clone())?;
        let by_id: HashMap<_, _> = folders.iter().map(|f| (&f.id, f)).collect();
        let mut rows = Vec::new();
        for folder in &folders {
            let mut parts = Vec::new();
            let mut current = folder;
            let mut visited = BTreeSet::new();
            loop {
                if !visited.insert(&current.id) || current.name.contains('\0') {
                    return Err(JmapError::Protocol.into());
                }
                parts.push(current.name.replace('%', "%25").replace('/', "%2F"));
                let Some(parent) = &current.parent_id else {
                    break;
                };
                current = by_id.get(parent).ok_or(JmapError::Protocol)?;
            }
            parts.reverse();
            rows.push(JmapFolder {
                remote: folder.id.clone(),
                path: parts.join("/"),
                role: folder.role.as_deref().and_then(MailboxRole::parse),
            });
        }
        archive.jmap_folders(account, &rows, field(&result, "state")?)?;
        Ok(())
    }
    pub fn sync(
        &self,
        account: &str,
        archive: &mut Archive,
        blobs: &FileBlobs,
        mut progress: impl FnMut(String),
        stop: impl Fn() -> bool,
    ) -> Result<()> {
        cancel(&stop)?;
        let (folders, state) = archive.bound_jmap_states(account, &self.account)?;
        // An interrupted initial scan may have encountered a newly created mailbox.
        if folders.is_none() || state.is_none() {
            self.refresh_folders(archive, account)?;
        }
        if archive.mail_edits_pending(account)? {
            self.flush_edits(account, archive, &stop)?;
        }
        let mut sync = Sync {
            client: self,
            account,
            archive,
            blobs,
            stop: &stop,
        };
        if state.is_none() {
            progress("Downloading Fastmail messages".into());
            sync.initial()?;
        }
        let mut rebuilt = false;
        loop {
            cancel(&stop)?;
            match sync.delta() {
                Ok(false) => break,
                Ok(true) => {},
                Err(Error::Jmap(JmapError::StateExpired)) if !rebuilt => {
                    rebuilt = true;
                    self.refresh_folders(sync.archive, account)?;
                    sync.initial()?;
                },
                Err(e) => return Err(e),
            }
        }
        sync.archive.reconcile_sent(account)?;
        Ok(())
    }
}

struct Sync<'a, F> {
    client: &'a Client,
    account: &'a str,
    archive: &'a mut Archive,
    blobs: &'a FileBlobs,
    stop: &'a F,
}
impl<F: Fn() -> bool> Sync<'_, F> {
    fn initial(&mut self) -> Result<()> {
        let baseline = self
            .client
            .method("Email/get", json!({"ids":[],"properties":["id"]}))?;
        let state = field(&baseline, "state")?;
        self.archive.begin_jmap_inventory()?;
        let mut position = 0;
        let mut query_state = None;
        loop {
            cancel(self.stop)?;
            let account = &self.client.account;
            let mut replies = self.client.calls(&[
                json!(["Email/query",{"accountId":account,"position":position,"limit":self.client.batch},"page"]),
                json!(["Email/get",{"accountId":account,"#ids":{"resultOf":"page","name":"Email/query","path":"/ids"},"properties":PROPERTIES},"messages"]),
            ], false)?.into_iter();
            let mut query = replies.next().ok_or(JmapError::Protocol)?;
            let result = replies.next().ok_or(JmapError::Protocol)?;
            let page = field(&query, "queryState")?;
            if query_state.as_deref().is_some_and(|s| s != page) {
                return Err(Error::Invalid(
                    "Mailbox changed during initial enumeration; sync again",
                ));
            }
            query_state = Some(page.to_owned());
            let ids: Vec<String> = decode(query["ids"].take())?;
            if query["position"].as_u64() != Some(position as u64) || ids.len() > self.client.batch
            {
                return Err(JmapError::Protocol.into());
            }
            self.apply(&ids, result, true)?;
            if ids.is_empty() {
                break;
            }
            position += ids.len();
        }
        self.archive
            .checkpoint_jmap(self.account, state, None, &[], true)?;
        Ok(())
    }
    fn delta(&mut self) -> Result<bool> {
        let (folders, email) = self.archive.jmap_states(self.account)?;
        let folders = folders.ok_or(JmapError::Protocol)?;
        let email = email.ok_or(JmapError::Protocol)?;
        let account = &self.client.account;
        let fetch = |path: &str, tag: &str| {
            json!([
            "Email/get",{"accountId":account,"properties":PROPERTIES,"#ids":{"resultOf":"changes","name":"Email/changes","path":path}},tag])
        };
        let responses = self.client.calls(&[

            json!(["Mailbox/changes",{"accountId":account,"sinceState":folders},"folders"]),
            json!(["Email/changes",{"accountId":account,"sinceState":email,"maxChanges":self.client.batch},"changes"]),
            fetch("/created","new"),fetch("/updated","updated")
        ],false)?;
        let mut replies = responses.into_iter();
        let folder_changes: Changes = decode(replies.next().ok_or(JmapError::Protocol)?)?;
        let changes: Changes = decode(replies.next().ok_or(JmapError::Protocol)?)?;
        if changes.old_state != email
            || folder_changes.old_state != folders
            || changes.has_more_changes && changes.old_state == changes.new_state
        {
            return Err(JmapError::Protocol.into());
        }
        let counts_only = folder_changes.created.is_empty()
            && folder_changes.destroyed.is_empty()
            && folder_changes
                .updated_properties
                .as_ref()
                .is_some_and(|properties| {
                    properties.iter().all(|p| {
                        matches!(
                            p.as_str(),
                            "totalEmails" | "unreadEmails" | "totalThreads" | "unreadThreads"
                        )
                    })
                });
        let folder_checkpoint = if folder_changes.new_state != folders && !counts_only {
            self.client.refresh_folders(self.archive, self.account)?;
            None
        } else {
            Some(folder_changes.new_state.as_str())
        };
        let mut destroyed = changes.destroyed;
        destroyed.extend(self.apply(
            &changes.created,
            replies.next().ok_or(JmapError::Protocol)?,
            false,
        )?);
        destroyed.extend(self.apply(
            &changes.updated,
            replies.next().ok_or(JmapError::Protocol)?,
            false,
        )?);
        if changes.new_state != email
            || folder_checkpoint.is_some_and(|new| new != folders)
            || !destroyed.is_empty()
        {
            self.archive.checkpoint_jmap(
                self.account,
                &changes.new_state,
                folder_checkpoint,
                &destroyed,
                false,
            )?;
        }
        Ok(changes.has_more_changes || (counts_only && folder_changes.has_more_changes))
    }
}
