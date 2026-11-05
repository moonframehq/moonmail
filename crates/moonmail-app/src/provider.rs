use {
    anyhow::{Context, Result, bail},
    moonframe::{Data, Handle, Native},
    moonmail_core::{
        BlobStore, MailDetail, MailPage, MailQuery, MailRepository, MailboxRole, MessageId,
        PageCursor, RelatedQuery, Relationship, SortField, ViewFilter,
    },
    moonmail_network::{Account, Auth, Keychain},
    moonmail_search::SqliteIndex,
    moonmail_store::{Archive, Draft, DraftCursor, FileBlobs},
    serde_json::json,
    std::{
        collections::{HashMap, HashSet},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
            mpsc::{self, Receiver},
        },
    },
};

#[path = "provider_accounts.rs"]
mod accounts;
#[path = "provider_attachments.rs"]
mod attachments;
#[cfg(feature = "capture")]
#[path = "provider_capture.rs"]
mod capture;
#[path = "provider_drafts.rs"]
mod drafts;
#[cfg(test)]
#[path = "provider_export_tests.rs"]
mod export_tests;
#[path = "provider_folders.rs"]
mod folders;
#[path = "provider_identities.rs"]
mod identities;
#[path = "provider_keywords.rs"]
mod keywords;
#[path = "provider_navigation.rs"]
mod navigation;
#[cfg(test)]
#[path = "provider_navigation_tests.rs"]
mod navigation_tests;
#[path = "provider_outbox.rs"]
mod outbox;
#[path = "provider_partitions.rs"]
mod partitions;
#[path = "provider_reader.rs"]
mod reader;
#[path = "provider_relationships.rs"]
mod relationships;
#[path = "provider_rules.rs"]
mod rules;
#[path = "provider_search.rs"]
mod search;
#[path = "provider_sync.rs"]
mod sync;

#[cfg(test)]
#[path = "provider_forward_tests.rs"]
mod forward_tests;
#[cfg(test)]
#[path = "provider_identity_tests.rs"]
mod identity_tests;
#[cfg(test)]
#[path = "provider_reader_tests.rs"]
mod reader_tests;
#[cfg(test)]
#[path = "provider_relationship_tests.rs"]
mod relationship_tests;

enum Event {
    Progress(String),
    Done(std::result::Result<(), String>, bool),
}

pub struct Mail {
    root: PathBuf,
    demo: bool,
    archive: Option<Archive>,
    index: Option<SqliteIndex>,
    query: MailQuery,
    search: String,
    saved_predicate: Option<moonmail_core::MailPredicate>,
    active_saved_search: Option<i64>,
    partition_selection: Option<(moonmail_store::PartitionField, String)>,
    partition_after: Option<String>,
    partition_panel: bool,
    prepared_search: Option<String>,
    page: MailPage,
    expanded: HashMap<MessageId, Option<PageCursor>>,
    checked: HashSet<MessageId>,
    selected: Option<MailDetail>,
    attachments: Vec<moonmail_core::Attachment>,
    attachment_message: Option<MessageId>,
    anchor: Option<MessageId>,
    related_before: Option<PageCursor>,
    related_sort: SortField,
    related_ascending: bool,
    relationship: Relationship,
    draft: Option<Draft>,
    draft_before: Option<DraftCursor>,
    outbox_before: Option<i64>,
    keyword_account: Option<String>,
    keyword_after: i64,
    rule_before: Option<i64>,
    identity_new_id: String,
    identity_account: Option<String>,
    job: Option<Receiver<Event>>,
    sync_cancel: Option<Arc<AtomicBool>>,
    sync_schedule: crate::sync_schedule::SyncSchedule,
    status: String,
    error: String,
    html_url: String,
    plain_url: String,
    reader_links: crate::reader_resources::ReaderLinks,
    reader_images: String,
    reader_dir: Option<tempfile::TempDir>,
    preferences: crate::fonts::Preferences,
    font_config: PathBuf,
    reader_revision: u64,
    font_families: Vec<String>,
    font_preview_url: String,
    font_preview_revision: u64,
    discovery: Option<crate::account_setup::Discovery>,
    discovery_job: Option<Receiver<std::result::Result<crate::account_setup::Discovery, String>>>,
    discovery_error: String,
}

impl Mail {
    pub fn new(root: PathBuf, demo: bool) -> Self {
        Self {
            root,
            demo,
            archive: None,
            index: None,
            query: MailQuery::default(),
            search: String::new(),
            saved_predicate: None,
            active_saved_search: None,
            partition_selection: None,
            partition_after: None,
            partition_panel: false,
            prepared_search: None,
            page: MailPage {
                rows: Vec::new(),
                next: None,
                first: None,
                last: None,
            },
            expanded: HashMap::new(),
            checked: HashSet::new(),
            selected: None,
            attachments: Vec::new(),
            attachment_message: None,
            anchor: None,
            related_before: None,
            related_sort: SortField::Date,
            related_ascending: false,
            relationship: Relationship::Correspondence,
            draft: None,
            draft_before: None,
            outbox_before: None,
            keyword_account: None,
            keyword_after: 0,
            rule_before: None,
            identity_new_id: uuid::Uuid::new_v4().to_string(),
            identity_account: None,
            job: None,
            sync_cancel: None,
            sync_schedule: crate::sync_schedule::SyncSchedule::default(),
            status: "Local mail, at your pace".into(),
            error: String::new(),
            html_url: String::new(),
            plain_url: String::new(),
            reader_links: crate::reader_resources::ReaderLinks::default(),
            reader_images: String::new(),
            reader_dir: None,
            preferences: crate::fonts::Preferences::default(),
            font_config: crate::fonts::path(demo),
            reader_revision: 0,
            font_families: Vec::new(),
            font_preview_url: String::new(),
            font_preview_revision: 0,
            discovery: None,
            discovery_job: None,
            discovery_error: String::new(),
        }
    }
    pub fn with_font_config(mut self, path: PathBuf) -> Self {
        self.font_config = path;
        self
    }
    fn archive(&self) -> Result<&Archive> {
        self.archive.as_ref().context("Archive is starting")
    }
    fn archive_mut(&mut self) -> Result<&mut Archive> {
        self.archive.as_mut().context("Archive is starting")
    }

    fn refresh(&mut self) -> Result<()> {
        if let Some(id) = self.active_saved_search {
            let (saved, mut predicate) = self.archive()?.resolve_search(id)?;
            let field = self.archive()?.search_partition(id)?;
            let partition = self
                .partition_selection
                .clone()
                .filter(|(selected, _)| field == Some(*selected));
            if let Some((field, value)) = &partition {
                predicate =
                    moonmail_core::MailPredicate::All(vec![predicate, field.predicate(value)]);
            }
            if self.saved_predicate.as_ref() != Some(&predicate) || self.search != saved.source {
                let panel = self.partition_panel;
                let result = self.set_search(saved.source, Some(predicate), false);
                self.active_saved_search = Some(id);
                self.partition_selection = partition;
                self.partition_panel = panel;
                return result;
            }
        }
        if self.search.trim().is_empty() {
            self.query.search_matches = false;
        } else {
            if self.prepared_search.as_deref() != Some(&self.search) {
                let predicate = match &self.saved_predicate {
                    Some(predicate) => predicate.clone(),
                    None => moonmail_search::parse_query(&self.search)?,
                };
                self.archive
                    .as_mut()
                    .context("Archive is starting")?
                    .prepare_predicate(
                        self.index.as_ref().context("Index is starting")?,
                        &predicate,
                    )?;
                self.prepared_search = Some(self.search.clone());
            }
            self.query.search_matches = true;
        }
        self.page = self.archive()?.page(&self.query, 75)?;
        self.expanded
            .retain(|id, _| self.page.rows.iter().any(|row| row.id == *id));
        Ok(())
    }

    fn index_pending(&mut self) -> Result<bool> {
        let (Some(archive), Some(index)) = (&self.archive, &mut self.index) else {
            return Ok(false);
        };
        // A failed journal acknowledgment can still follow a successful index
        // commit, so discard the cached match set on errors as well.
        let result = crate::local::index_batch(archive, index);
        if !matches!(result, Ok(false)) {
            self.prepared_search = None;
        }
        result
    }

    fn select(&mut self, id: MessageId, anchor: bool) -> Result<()> {
        self.reader_revision = self.reader_revision.wrapping_add(1);
        if self.preferences.mark_read && !self.archive()?.has_pending_move(id)? {
            self.archive_mut()?.set_flag(id, "seen", true)?;
        }
        let detail = self.archive()?.detail(id)?;
        let links = crate::reader_resources::links(&detail.text, detail.html.as_deref())?;
        let dir = self
            .reader_dir
            .as_ref()
            .context("Reader directory is missing")?;
        let path = dir.path().join(format!("plain-{}.html", id.0 % 2));
        std::fs::write(
            &path,
            crate::page::plain_with_font(&detail.text, &self.preferences.reader),
        )?;
        self.plain_url = url::Url::from_file_path(path)
            .map_err(|_| anyhow::anyhow!("Invalid reader path"))?
            .to_string();
        self.plain_url.push_str(&format!(
            "?message={}&revision={}",
            id.0, self.reader_revision
        ));
        self.html_url.clear();
        if let Some(html) = &detail.html {
            let dir = self
                .reader_dir
                .as_ref()
                .context("Reader directory is missing")?;
            // Two rotating files keep disk use bounded while changing the URL.
            let path = dir.path().join(format!("message-{}.html", id.0 % 2));
            std::fs::write(
                &path,
                crate::page::html_with_font(html, &self.preferences.reader),
            )?;
            self.html_url = url::Url::from_file_path(path)
                .map_err(|_| anyhow::anyhow!("Invalid reader path"))?
                .to_string();
            self.html_url.push_str(&format!(
                "?message={}&revision={}",
                id.0, self.reader_revision
            ));
        }
        self.selected = Some(detail);
        self.reader_links = links;
        self.reader_images.clear();
        self.attachments.clear();
        self.attachment_message = None;
        if anchor {
            if self.anchor != Some(id) {
                self.related_before = None;
            }
            self.anchor = Some(id);
        }
        self.refresh()
    }

    fn snapshot(&self) -> Result<serde_json::Value> {
        let archive = self.archive()?;
        let outbox = archive.outbox_page(self.outbox_before, 50)?;
        let related = self
            .related_query()
            .map(|query| archive.related_page(&query, 50))
            .transpose()?;
        let mailboxes = archive.mailboxes()?;
        use std::hash::{Hash, Hasher};
        let mut list_hash = std::hash::DefaultHasher::new();
        format!("{:?}-{}", self.query, self.search).hash(&mut list_hash);
        serde_json::to_string(&self.preferences.columns)?.hash(&mut list_hash);
        let mut expanded: Vec<_> = self
            .expanded
            .iter()
            .map(|(id, cursor)| (id.0, cursor.as_ref().map(|cursor| cursor.id.0)))
            .collect();
        expanded.sort_unstable();
        expanded.hash(&mut list_hash);
        let mut snapshot = json!({
            "rows":self.listing()?,"more":self.page.next.is_some(),"previous":self.query.before.is_some(),"mailboxes":mailboxes,
            "folder_tree":crate::navigation::folders(&mailboxes),
            "sort":self.query.sort,"ascending":self.query.ascending,"threaded":self.query.threaded,
            "page_count":self.page.rows.len(),
            "checked_count":self.checked.len(),
            "move_operations":archive.move_operations()?,
            "unresolved_operations":archive.unresolved_move_count()? + archive.unresolved_folder_count()? + archive.unresolved_keyword_count()?,
            "selected_moving":self.selected.as_ref().map(|detail|archive.has_pending_move(detail.summary.id)).transpose()?.unwrap_or(false),
            "list_key":format!("{:016x}",list_hash.finish()),"anchor":self.anchor,
            "selected":self.selected.as_ref().map(|detail|json!({"summary":detail.summary,"html":detail.html.is_some()})),"related":related,
            "mailbox":self.query.mailbox,"search":self.search,"draft":self.draft,"outbox":outbox.rows,
            "drafts":archive.drafts(self.draft_before.as_ref(),50)?,"draft_count":archive.draft_count()?,
            "attachments":self.attachments,"attachment_message":self.attachment_message,
            "filter":match self.query.filter {ViewFilter::All=>"all",ViewFilter::Unread=>"unread",ViewFilter::Starred=>"starred"},
            "account":Account::load(&self.root)?.map(|account|account.email),"demo":self.demo,
            "busy":self.job.is_some(),"status":self.status,"error":self.error,"html_url":self.html_url,"plain_url":self.plain_url,
            "root":self.root.display().to_string(),
            "font_config":self.font_config.display().to_string(),
            "preferences":self.preferences,
            "font_families":self.font_families,"font_preview_url":self.font_preview_url,
            "discovery":self.discovery,"discovering":self.discovery_job.is_some(),"discovery_error":self.discovery_error,
        });
        snapshot["outbox_more"] = json!(outbox.next.is_some());
        self.sender_snapshot(&mut snapshot)?;
        if let Some(rows) = snapshot["drafts"]["rows"].as_array_mut() {
            for row in rows {
                row["updated_label"] = json!(reader::date_label(
                    row["updated"]
                        .as_i64()
                        .map(|milliseconds| milliseconds / 1000)
                ));
            }
        }
        snapshot["folder_operations"] = json!(archive.folder_operations()?);
        self.keyword_snapshot(&mut snapshot)?;
        self.rule_snapshot(&mut snapshot)?;
        self.partition_snapshot(&mut snapshot)?;
        snapshot["columns_key"] = json!(serde_json::to_string(&self.preferences.columns)?);
        snapshot["outbox_count"] = json!(archive.outbox_count()?);
        snapshot["syncing"] = json!(self.sync_cancel.is_some());
        snapshot["sync_schedule"] = json!(self.sync_schedule.label());
        snapshot["saved_searches"] = json!(archive.saved_searches()?);
        snapshot["active_saved_search"] = json!(self.active_saved_search);
        let move_selection = self.move_selection();
        snapshot["move_account"] = json!(self.move_account(&move_selection)?);
        snapshot["move_selection"] = json!(move_selection);
        snapshot["reader_links"] = json!(self.reader_links);
        snapshot["reader_images"] = json!(self.reader_images);
        snapshot["thread_warnings"] = json!(
            self.selected
                .as_ref()
                .map(|selected| archive.thread_warnings(selected.summary.id))
                .transpose()?
                .unwrap_or_default()
        );
        snapshot["received_date"] = json!(reader::date_label(
            self.selected.as_ref().and_then(|detail| detail.received_at)
        ));
        snapshot["date_header"] = json!(
            self.selected
                .as_ref()
                .is_some_and(|detail| detail.header_date.is_some())
        );
        let account = self.identity_settings_account()?;
        snapshot["identities"] = json!(archive.identities(&account)?);
        snapshot["draft_identities"] = json!(
            self.draft
                .as_ref()
                .map(|draft| archive.identities(&draft.account))
                .transpose()?
                .unwrap_or_default()
        );
        snapshot["identity_account"] = json!(account);
        snapshot["identity_new_id"] = json!(self.identity_new_id);
        snapshot["related_sort"] = json!(self.related_sort);
        snapshot["related_ascending"] = json!(self.related_ascending);
        snapshot["related_thread"] = json!(self.relationship == Relationship::Thread);
        snapshot["related_key"] = json!(format!(
            "{:x}-{:x}-{:?}-{}",
            self.anchor.map_or(0, |id| id.0),
            self.related_before.as_ref().map_or(0, |cursor| cursor.id.0),
            self.related_sort,
            u8::from(self.related_ascending)
        ));
        Ok(snapshot)
    }

    fn listing(&self) -> Result<Vec<serde_json::Value>> {
        let mut rows = Vec::new();
        for summary in &self.page.rows {
            let expanded = self.query.threaded && self.expanded.contains_key(&summary.id);
            let mut row = serde_json::to_value(summary)?;
            row["expanded"] = json!(expanded);
            row["depth"] = json!(0);
            rows.push(row);
            if expanded {
                let query = self.expanded_query(summary.id);
                let children = self.archive()?.page(&query, 100)?;
                if children.next.is_some() || query.before.is_some() {
                    rows.push(json!({"id":summary.id,"thread_pager":true,"thread_more":children.next.is_some(),"thread_cursor":query.before.as_ref().map_or(0, |cursor|cursor.id.0),"depth":1}));
                }
                for child in children
                    .rows
                    .into_iter()
                    .filter(|child| child.id != summary.id)
                {
                    let mut row = serde_json::to_value(child)?;
                    row["depth"] = json!(1);
                    rows.push(row);
                }
            }
        }
        for row in &mut rows {
            row["checked"] = json!(
                row["id"]
                    .as_u64()
                    .is_some_and(|id| self.checked.contains(&MessageId(id)))
            );
        }
        Ok(rows)
    }

    fn publish(&self, host: &Handle) -> Result<()> {
        host.publish_json(&self.snapshot()?);
        Ok(())
    }

    fn launch(
        &mut self,
        operation: impl FnOnce(PathBuf, mpsc::SyncSender<Event>) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        if self.demo {
            bail!("Demo mode is offline. Launch `just run` to connect an account.");
        }
        if self.job.is_some() {
            bail!("A network operation is already running");
        }
        let (sender, receiver) = mpsc::sync_channel(32);
        let root = self.root.clone();
        std::thread::Builder::new()
            .name("moonmail-network".into())
            .spawn(move || {
                let result = operation(root, sender.clone());
                let retryable = result
                    .as_ref()
                    .err()
                    .and_then(|error| error.downcast_ref::<moonmail_network::Error>())
                    .is_some_and(moonmail_network::Error::retryable_sync);
                let _ = sender.send(Event::Done(
                    result.map_err(|error| format!("{error:#}")),
                    retryable,
                ));
            })?;
        self.job = Some(receiver);
        self.status = "Working…".into();
        Ok(())
    }

    fn handle(&mut self, message: Data) -> Result<()> {
        let text = |key| {
            message
                .get(key)
                .and_then(Data::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let action = text("action");
        if matches!(
            action.as_str(),
            "folder" | "filter" | "next" | "previous" | "first" | "sort" | "threaded" | "expand"
        ) {
            self.checked.clear();
        }
        match action.as_str() {
            "partition_close" => self.partition_panel = false,
            "partition_open" | "partition_field" | "partition_first" | "partition_next"
            | "partition_choose" => self.handle_partition(&action, &message)?,
            "rule_save" | "rule_delete" | "rule_apply" | "rule_first" | "rule_next" => {
                self.handle_rule(&action, &message)?
            },
            "keyword_set" | "keyword_label" | "keyword_account" | "keyword_next"
            | "keyword_first" | "keyword_retry" | "keyword_discard" => {
                self.handle_keyword(&action, &message)?
            },
            "folder_create" | "folder_rename" | "folder_delete" | "folder_cancel"
            | "folder_dismiss" => self.handle_folder_edit(&action, &message)?,
            "seek_unread" | "seek_thread" => {
                self.seek_message(text("value") == "up", action == "seek_unread")?;
            },
            "thread_read" | "thread_unread" => {
                let id = self
                    .selected
                    .as_ref()
                    .context("Select a message first")?
                    .summary
                    .id;
                if let Some(expected) = message.get("id").and_then(Data::as_i64) {
                    anyhow::ensure!(expected == id.0 as i64, "Thread selection changed");
                }
                let count = self
                    .archive_mut()?
                    .set_thread_seen(id, action == "thread_read")?;
                self.selected = Some(self.archive()?.detail(id)?);
                self.refresh()?;
                self.status = format!(
                    "{count} thread messages marked {} across all folders",
                    if action == "thread_read" {
                        "read"
                    } else {
                        "unread"
                    }
                );
            },
            "selection_flag" => {
                let ids = self.move_selection();
                self.validate_move_choice(&message, &ids)?;
                let value = message
                    .get("value")
                    .and_then(Data::as_bool)
                    .context("Choose a flag value")?;
                let count = self.archive_mut()?.set_flags(&ids, &text("kind"), value)?;
                self.refresh()?;
                // Refresh displayed flags without turning an explicit unread
                // command back into a read through reader-selection side effects.
                if let Some(detail) = &self.selected {
                    self.selected = Some(self.archive()?.detail(detail.summary.id)?);
                }
                self.status =
                    format!("Updated {count} selected message(s); remote flags upload on Sync");
            },
            "toggle_selection" => {
                let id = MessageId(
                    message
                        .get("id")
                        .and_then(Data::as_i64)
                        .filter(|id| *id > 0)
                        .context("Choose a visible message")? as u64,
                );
                if !self
                    .listing()?
                    .iter()
                    .any(|row| row["id"].as_u64() == Some(id.0))
                {
                    bail!("Choose a visible message");
                }
                if !self.checked.remove(&id) {
                    if self.checked.len() >= 100 {
                        bail!("Select at most 100 messages at a time");
                    }
                    self.checked.insert(id);
                }
            },
            "clear_selection" => self.checked.clear(),
            "archive_messages" | "trash_messages" | "move_messages" | "choose_move"
            | "choose_copy" | "copy_messages" => {
                let ids = self.move_selection();
                anyhow::ensure!(!ids.is_empty(), "Select a message first");
                if action == "choose_move" || action == "choose_copy" {
                    self.validate_move_choice(&message, &ids)?;
                }
                let target = match action.as_str() {
                    "archive_messages" => moonmail_store::MoveTarget::Role(MailboxRole::Archive),
                    "trash_messages" => moonmail_store::MoveTarget::Role(MailboxRole::Trash),
                    _ => moonmail_store::MoveTarget::Folder(
                        message
                            .get("id")
                            .and_then(Data::as_i64)
                            .context("Choose a destination folder")?,
                    ),
                };
                let source = self.query.mailbox;
                let copying = action == "choose_copy" || action == "copy_messages";
                let count = if copying {
                    self.archive_mut()?.queue_copies(&ids, source, target)?
                } else {
                    self.archive_mut()?.queue_moves(&ids, source, target)?
                };
                self.checked.clear();
                self.refresh()?;
                self.status = format!(
                    "{count} message(s): local {} applied; remote operations queued for next Sync",
                    if copying { "copies" } else { "moves" }
                );
            },
            "folder_role" => {
                let id = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .context("Choose a folder")?;
                let role = match text("role").as_str() {
                    "archive" => Some(MailboxRole::Archive),
                    "trash" => Some(MailboxRole::Trash),
                    "sent" => Some(MailboxRole::Sent),
                    "junk" => Some(MailboxRole::Junk),
                    "default" => None,
                    _ => bail!("Choose Archive, Trash, Sent, Junk, or server default"),
                };
                self.archive_mut()?.set_mailbox_role(id, role)?;
                self.status = "Folder role saved".into();
            },
            "cancel_move" | "dismiss_move" => {
                let id = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .context("Choose a mail operation")?;
                if action == "cancel_move" {
                    self.archive()?.cancel_move(id)?;
                } else {
                    self.archive()?.dismiss_unknown_move(id)?;
                }
                self.status = "Mail operation updated".into();
            },
            "fonts_load" => {
                if self.font_families.is_empty() {
                    self.font_families = crate::fonts::installed();
                }
            },
            "font_preview" => {
                let font = crate::fonts::ReaderFont {
                    family: text("family"),
                    size: text("size").parse().context("Enter a font size")?,
                };
                let html = crate::fonts::preview(&font)?;
                self.font_preview_revision = self.font_preview_revision.wrapping_add(1);
                let path = self
                    .reader_dir
                    .as_ref()
                    .context("Preview directory is missing")?
                    .path()
                    .join(format!("font-{}.html", self.font_preview_revision % 2));
                std::fs::write(&path, html)?;
                self.font_preview_url = format!(
                    "{}?revision={}",
                    url::Url::from_file_path(path)
                        .map_err(|_| anyhow::anyhow!("Invalid preview path"))?,
                    self.font_preview_revision
                );
            },
            "account_discover" => {
                anyhow::ensure!(
                    self.discovery_job.is_none(),
                    "Account discovery is already running"
                );
                let email = text("email").trim().to_owned();
                moonmail_network::validate_address(&email)?;
                self.discovery = None;
                self.discovery_error.clear();
                let (sender, receiver) = mpsc::sync_channel(1);
                let lookup_mx = !self.demo;
                std::thread::Builder::new()
                    .name("moonmail-discovery".into())
                    .spawn(move || {
                        let _ = sender.send(
                            crate::account_setup::discover(&email, lookup_mx)
                                .map_err(|error| format!("{error:#}")),
                        );
                    })?;
                self.discovery_job = Some(receiver);
            },
            "folder" => {
                if let Some(id) = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .filter(|id| *id > 0)
                {
                    anyhow::ensure!(
                        self.archive()?
                            .mailboxes()?
                            .iter()
                            .any(|folder| folder.id == id && folder.selectable),
                        "This folder is no longer available"
                    );
                }
                self.active_saved_search = None;
                self.saved_predicate = None;
                self.query.before = None;
                self.query.filter = ViewFilter::All;
                self.expanded.clear();
                self.search.clear();
                self.query.mailbox = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .filter(|id| *id > 0);
                self.refresh()?;
            },
            "filter" => {
                self.expanded.clear();
                self.query.before = None;
                self.query.filter = match text("value").as_str() {
                    "unread" => ViewFilter::Unread,
                    "starred" => ViewFilter::Starred,
                    _ => ViewFilter::All,
                };
                self.refresh()?;
            },
            "search" => {
                self.set_search(text("value"), None, false)?;
            },
            "search_save" | "search_open" | "search_delete" => {
                self.handle_search(&action, &message)?
            },
            "next" => {
                self.next_page()?;
            },
            "previous" => {
                self.previous_page()?;
            },
            "first" => {
                self.expanded.clear();
                self.query.before = None;
                self.refresh()?;
            },
            "sort" => {
                let sort = match text("field").as_str() {
                    "sender" => SortField::Sender,
                    "subject" => SortField::Subject,
                    _ => SortField::Date,
                };
                self.query.ascending = if sort == self.query.sort {
                    !self.query.ascending
                } else {
                    sort != SortField::Date
                };
                self.query.sort = sort;
                self.query.before = None;
                self.refresh()?;
            },
            "threaded" => {
                self.query.threaded = !self.query.threaded;
                self.preferences.threaded = self.query.threaded;
                self.preferences.save(&self.root)?;
                self.query.before = None;
                self.expanded.clear();
                self.refresh()?;
            },
            "expand" => {
                if let Some(id) = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .filter(|id| *id > 0)
                {
                    let id = MessageId(id as u64);
                    if self
                        .page
                        .rows
                        .iter()
                        .any(|row| row.id == id && row.thread_count > 1)
                        && self.expanded.remove(&id).is_none()
                    {
                        self.expanded.insert(id, None);
                    }
                }
            },
            "thread_first" | "thread_next" => self.handle_thread_page(&action, &message)?,
            "columns_edit" => {
                anyhow::ensure!(
                    text("base") == serde_json::to_string(&self.preferences.columns)?,
                    "Column layout changed; use the refreshed controls"
                );
                let mut settings = self.preferences.clone();
                settings.columns = settings.columns.edit(
                    &text("operation"),
                    &text("field"),
                    message.get("width").and_then(Data::as_i64),
                )?;
                settings.save(&self.root)?;
                self.preferences = settings;
                self.status = "Column layout saved".into();
            },
            "settings_save" => {
                let settings = crate::fonts::Preferences {
                    interface_family: text("interface_family").trim().to_owned(),
                    list_family: text("list_family").trim().to_owned(),
                    font_size: text("font_size")
                        .parse()
                        .context("Enter an interface font size between 10 and 16")?,
                    reader: crate::fonts::ReaderFont {
                        family: text("reader_family").trim().to_owned(),
                        size: text("reader_size")
                            .parse()
                            .context("Enter a reader font size between 8 and 32")?,
                    },
                    threaded: message
                        .get("threaded")
                        .and_then(Data::as_bool)
                        .unwrap_or(false),
                    mark_read: message
                        .get("mark_read")
                        .and_then(Data::as_bool)
                        .unwrap_or(true),
                    sync_minutes: if text("sync_minutes").is_empty() {
                        self.preferences.sync_minutes
                    } else {
                        text("sync_minutes")
                            .parse()
                            .context("Invalid sync interval")?
                    },
                    // A client that does not know about icons keeps the saved one.
                    app_icon: match text("app_icon").as_str() {
                        "" => self.preferences.app_icon,
                        name => crate::app_icon::AppIcon::parse(name)?,
                    },
                    columns: self.preferences.columns.clone(),
                };
                settings.validate()?;
                settings.apply_fonts(&self.font_config)?;
                settings.save(&self.root)?;
                self.sync_schedule
                    .configure(settings.sync_minutes, std::time::Instant::now());
                self.query.threaded = settings.threaded;
                self.query.before = None;
                self.expanded.clear();
                self.preferences = settings;
                // Re-render without treating a preference change as reading mail.
                if let Some(id) = self.selected.as_ref().map(|detail| detail.summary.id) {
                    let mark_read = self.preferences.mark_read;
                    self.preferences.mark_read = false;
                    let result = self.select(id, false);
                    self.preferences.mark_read = mark_read;
                    result?;
                } else {
                    self.refresh()?;
                }
                self.status = "Settings saved".into();
            },
            "select" | "related" => {
                if let Some(id) = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .filter(|id| *id > 0)
                {
                    self.select(MessageId(id as u64), action == "select")?;
                }
            },
            "move" => {
                self.navigate_message(text("value") == "up")?;
            },
            "star" | "unread" => {
                if let Some(detail) = &self.selected {
                    let id = detail.summary.id;
                    let value = if action == "star" {
                        !detail.summary.starred
                    } else {
                        false
                    };
                    self.archive_mut()?.set_flag(
                        id,
                        if action == "star" { "flagged" } else { "seen" },
                        value,
                    )?;
                    self.selected = Some(self.archive()?.detail(id)?);
                    self.refresh()?;
                }
            },
            "compose" | "reply" | "reply_all" | "forward" | "forward_text" | "draft"
            | "draft_open" | "draft_close" | "draft_discard" | "drafts_next" | "drafts_first" => {
                self.handle_draft(&action, &message)?
            },
            "attachments" | "attachment_save" | "message_save" | "messages_save"
            | "draft_attach" | "draft_detach" => self.handle_attachment(&action, &message)?,
            "reader_link" | "reader_images" => self.handle_reader(&action, &message)?,
            "related_first" | "related_next" | "related_sort" | "related_mode" => {
                self.handle_relationship(&action, &message)?
            },
            "identity_save" | "identity_default" | "identity_delete" | "identity_account" => {
                self.handle_identity(&action, &message)?
            },
            "draft_identity" | "draft_signature" | "draft_account" => {
                self.handle_draft(&action, &message)?
            },
            "outbox_first" | "outbox_next" | "outbox_cancel" | "outbox_edit" | "outbox_retry"
            | "outbox_sent" => self.handle_outbox(&action, &message)?,
            "queue" | "queue_only" => {
                if self.demo && action == "queue" {
                    bail!("Sending is disabled in the synthetic demo");
                }
                if self.job.is_some() && action == "queue" {
                    bail!("Wait for the current network operation to finish");
                }
                let draft = self.draft.clone().context("Compose a message first")?;
                anyhow::ensure!(
                    draft.id == text("id"),
                    "This compose view is stale; reopen the intended draft"
                );
                let account = if self.demo && draft.account == "you@example.com" {
                    Account::fastmail("you@example.com")?
                } else {
                    Account::load_for(&self.root, &draft.account)?.context(
                        "Reconnect this draft's sending account or choose another From account",
                    )?
                };
                let (raw, recipients) = moonmail_network::prepare_message_with_attachments(
                    &account,
                    &draft,
                    &FileBlobs::attachments(&self.root),
                )?;
                let blob = FileBlobs::new(&self.root).put(&raw)?;
                let parsed = moonmail_core::MimeParser::parse(
                    &moonmail_mime::StalwartParser::default(),
                    &raw,
                )?;
                let id = self.archive_mut()?.queue_with_message_id(
                    &draft,
                    &account.email,
                    &blob,
                    &recipients,
                    parsed.message_id.as_deref(),
                )?;
                self.draft = None;
                self.outbox_before = None;
                if action == "queue" {
                    self.launch(move |root, _| Ok(moonmail_network::send_queued(&root, id)?))?;
                } else {
                    self.status = "Queued locally. Open Outbox to send, edit, or cancel.".into();
                }
            },
            "send" => {
                let id = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .context("Missing outbox id")?;
                self.launch(move |root, _| Ok(moonmail_network::send_queued(&root, id)?))?;
            },
            "sync" => {
                self.start_sync(true)?;
            },
            "cancel_sync" => {
                self.sync_schedule.pause();
                if let Some(cancel) = &self.sync_cancel {
                    cancel.store(true, Ordering::Relaxed);
                    self.status = "Stopping Sync after the current command…".into();
                }
            },
            "login" | "account_password" | "account_check" | "oauth_client_import" => {
                self.handle_account(&action, &message)?
            },
            "disconnect" => {
                let email = text("account");
                let account =
                    Account::load_for(&self.root, &email)?.context("Choose a connected account")?;
                self.launch(move |root, _| {
                    Auth::new(Keychain).disconnect(&account.email)?;
                    Account::remove(&root, &account.email)?;
                    Ok(())
                })?;
            },
            "reindex" => {
                self.archive()?.reset_index()?;
                self.status = "Rebuilding search in bounded batches…".into();
            },
            _ => {},
        }
        Ok(())
    }
}

impl Mail {
    fn initialize(&mut self) -> Result<()> {
        let archive = Archive::open(&self.root)?;
        self.preferences = crate::fonts::Preferences::load(&self.root)?;
        self.sync_schedule
            .configure(self.preferences.sync_minutes, std::time::Instant::now());
        self.query.threaded = self.preferences.threaded;
        self.index = Some(crate::local::open_index(&self.root, &archive)?);
        self.draft = archive.latest_draft()?;
        self.archive = Some(archive);
        self.reader_dir = Some(
            tempfile::Builder::new()
                .prefix("reader-")
                .tempdir_in(&self.root)?,
        );
        self.refresh()?;
        if let Some(row) = self.page.rows.first() {
            self.select(row.id, true)?;
        }
        Ok(())
    }
}

impl Native for Mail {
    fn start(&mut self, host: &Handle) -> std::result::Result<(), String> {
        self.initialize()
            .and_then(|()| self.publish(host))
            .map_err(|error| format!("{error:#}"))
    }
    fn receive(&mut self, host: &Handle, _from: &str, message: Data) {
        self.error.clear();
        let window = self.composer_window_event(&message);
        if let Err(error) = self.handle(message) {
            self.error = format!("{error:#}");
        }
        if let Err(error) = self.publish(host) {
            host.fail(format!("{error:#}"));
        } else if self.error.is_empty()
            && let Some(event) = window
        {
            host.send("mail-composer", event.into());
        }
    }
    fn tick(&mut self, host: &Handle) {
        let mut changed = false;
        let mut done = false;
        if let Some(receiver) = &self.discovery_job {
            match receiver.try_recv() {
                Ok(result) => {
                    match result {
                        Ok(discovery) => self.discovery = Some(discovery),
                        Err(error) => self.discovery_error = error,
                    }
                    self.discovery_job = None;
                    changed = true;
                },
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.discovery_error =
                        "Provider lookup stopped. Choose from the list or try again.".into();
                    self.discovery_job = None;
                    changed = true;
                },
                Err(mpsc::TryRecvError::Empty) => {},
            }
        }
        if let Some(receiver) = &self.job {
            loop {
                let event = match receiver.try_recv() {
                    Ok(event) => event,
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if !done {
                            self.error =
                                "Network worker stopped unexpectedly; use Sync to retry".into();
                            self.sync_schedule.pause();
                            changed = true;
                            done = true;
                        }
                        break;
                    },
                };
                changed = true;
                match event {
                    Event::Progress(status) => self.status = status,
                    Event::Done(result, retryable) => {
                        done = true;
                        if self.sync_cancel.is_some() {
                            self.sync_schedule.finished(
                                result.is_ok(),
                                retryable,
                                std::time::Instant::now(),
                            );
                        }
                        match result {
                            Ok(()) => {
                                self.status = if self.sync_cancel.is_some() {
                                    "Up to date"
                                } else {
                                    "Operation completed"
                                }
                                .into()
                            },
                            Err(error) => {
                                self.status = "Operation stopped".into();
                                self.error = error;
                            },
                        }
                    },
                }
            }
        }
        if done {
            self.job = None;
            self.sync_cancel = None;
        }
        match self.maybe_auto_sync(std::time::Instant::now()) {
            Ok(started) => changed |= started,
            Err(error) => {
                self.error = format!("{error:#}");
                self.sync_schedule.pause();
                changed = true;
            },
        }
        match self.index_pending() {
            Ok(indexed) => changed |= indexed,
            Err(error) => {
                self.error = format!("{error:#}");
                changed = true;
            },
        }
        if changed && let Err(error) = self.refresh().and_then(|()| self.publish(host)) {
            host.fail(format!("{error:#}"));
        }
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
