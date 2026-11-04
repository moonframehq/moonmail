use {
    anyhow::{Context, Result, bail},
    moonframe::{Data, Handle, Native},
    moonmail_core::{
        BlobStore, MailDetail, MailPage, MailQuery, MailRepository, MessageId, SearchIndex,
        SearchQuery, SortField, ViewFilter,
    },
    moonmail_network::{Account, Auth, Keychain},
    moonmail_search::SqliteIndex,
    moonmail_store::{Archive, Draft, FileBlobs},
    serde_json::json,
    std::{
        collections::HashSet,
        num::NonZeroUsize,
        path::PathBuf,
        sync::mpsc::{self, Receiver},
    },
};

enum Event {
    Progress(String),
    Done(std::result::Result<(), String>),
}

pub struct Mail {
    root: PathBuf,
    demo: bool,
    archive: Option<Archive>,
    index: Option<SqliteIndex>,
    query: MailQuery,
    search: String,
    page: MailPage,
    expanded: HashSet<MessageId>,
    selected: Option<MailDetail>,
    anchor: Option<MessageId>,
    draft: Option<Draft>,
    job: Option<Receiver<Event>>,
    status: String,
    error: String,
    html_url: String,
    plain_url: String,
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
            page: MailPage {
                rows: Vec::new(),
                next: None,
            },
            expanded: HashSet::new(),
            selected: None,
            anchor: None,
            draft: None,
            job: None,
            status: "Local mail, at your pace".into(),
            error: String::new(),
            html_url: String::new(),
            plain_url: String::new(),
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
        if self.search.trim().is_empty() {
            self.query.ids = None;
        } else {
            let phrases: Vec<_> = self.search.split_whitespace().collect();
            self.query.ids = Some(self.index.as_ref().context("Index is starting")?.search(
                SearchQuery {
                    phrases: &phrases,
                    limit: NonZeroUsize::new(1000).context("Invalid limit")?,
                },
            )?);
        }
        self.page = self.archive()?.page(&self.query, 75)?;
        Ok(())
    }

    fn select(&mut self, id: MessageId, anchor: bool) -> Result<()> {
        self.reader_revision = self.reader_revision.wrapping_add(1);
        if self.preferences.mark_read {
            self.archive_mut()?.set_flag(id, "seen", true)?;
        }
        let detail = self.archive()?.detail(id)?;
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
        if anchor {
            self.anchor = Some(id);
        }
        self.refresh()
    }

    fn snapshot(&self) -> Result<serde_json::Value> {
        let archive = self.archive()?;
        let correspondence = self
            .anchor
            .map(|id| archive.correspondence(id, 100))
            .transpose()?
            .unwrap_or_default();
        let thread = self
            .anchor
            .map(|id| archive.thread(id, 100))
            .transpose()?
            .unwrap_or_default();
        let mailboxes = archive.mailboxes()?;
        Ok(json!({
            "rows":self.listing()?,"more":self.page.next.is_some(),"mailboxes":mailboxes,
            "folder_tree":crate::navigation::folders(&mailboxes),
            "sort":self.query.sort,"ascending":self.query.ascending,"threaded":self.query.threaded,
            "page_count":self.page.rows.len(),
            "list_key":format!("{:?}-{:?}-{:?}-{}-{:?}-{}-{}",self.query.mailbox,self.query.filter,self.query.before,self.search,self.query.sort,self.query.ascending,self.query.threaded),"anchor":self.anchor,
            "selected":self.selected.as_ref().map(|detail|json!({"summary":detail.summary,"html":detail.html.is_some()})),"correspondence":correspondence,"thread":thread,
            "mailbox":self.query.mailbox,"search":self.search,"draft":self.draft,"outbox":archive.outbox()?,
            "filter":match self.query.filter {ViewFilter::All=>"all",ViewFilter::Unread=>"unread",ViewFilter::Starred=>"starred"},
            "account":Account::load(&self.root)?.map(|account|account.email),"demo":self.demo,
            "busy":self.job.is_some(),"status":self.status,"error":self.error,"html_url":self.html_url,"plain_url":self.plain_url,
            "search_capped":self.query.ids.as_ref().is_some_and(|ids|ids.len()==1000),"root":self.root.display().to_string(),
            "font_config":self.font_config.display().to_string(),
            "preferences":self.preferences,
            "font_families":self.font_families,"font_preview_url":self.font_preview_url,
            "discovery":self.discovery,"discovering":self.discovery_job.is_some(),"discovery_error":self.discovery_error,
        }))
    }

    fn listing(&self) -> Result<Vec<serde_json::Value>> {
        let mut rows = Vec::new();
        for summary in &self.page.rows {
            let expanded = self.query.threaded && self.expanded.contains(&summary.id);
            let mut row = serde_json::to_value(summary)?;
            row["expanded"] = json!(expanded);
            row["depth"] = json!(0);
            rows.push(row);
            if expanded {
                let mut query = self.query.clone();
                query.threaded = false;
                query.thread = Some(summary.id);
                query.before = None;
                query.sort = SortField::Date;
                query.ascending = true;
                let children = self.archive()?.page(&query, 100)?;
                for child in children
                    .rows
                    .into_iter()
                    .filter(|child| child.id != summary.id)
                    .take(99)
                {
                    let mut row = serde_json::to_value(child)?;
                    row["depth"] = json!(1);
                    rows.push(row);
                }
                if summary.thread_count > 100
                    && let Some(row) = rows.last_mut()
                {
                    row["thread_capped"] = json!(true);
                }
            }
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
                let result = operation(root, sender.clone()).map_err(|error| format!("{error:#}"));
                let _ = sender.send(Event::Done(result));
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
        match action.as_str() {
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
                self.expanded.clear();
                self.search = text("value");
                self.query.before = None;
                self.refresh()?;
            },
            "next" => {
                if self.page.next.is_some() {
                    self.query.before = self.page.next.clone();
                    self.expanded.clear();
                    self.refresh()?;
                }
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
                        && !self.expanded.remove(&id)
                    {
                        self.expanded.insert(id);
                    }
                }
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
                    // A client that does not know about icons keeps the saved one.
                    app_icon: match text("app_icon").as_str() {
                        "" => self.preferences.app_icon,
                        name => crate::app_icon::AppIcon::parse(name)?,
                    },
                };
                settings.validate()?;
                settings.apply_fonts(&self.font_config)?;
                settings.save(&self.root)?;
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
                let visible = self.listing()?;
                let current = self
                    .selected
                    .as_ref()
                    .and_then(|detail| {
                        visible
                            .iter()
                            .position(|row| row["id"].as_u64() == Some(detail.summary.id.0))
                    })
                    .unwrap_or(0);
                let next = if text("value") == "up" {
                    current.saturating_sub(1)
                } else {
                    (current + 1).min(visible.len().saturating_sub(1))
                };
                if let Some(id) = visible.get(next).and_then(|row| row["id"].as_u64()) {
                    self.select(MessageId(id), true)?;
                }
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
            "compose" | "reply" => {
                if self.draft.is_none() {
                    let mut draft = Draft {
                        id: uuid::Uuid::new_v4().to_string(),
                        ..Default::default()
                    };
                    if action == "reply"
                        && let Some(detail) = &self.selected
                    {
                        draft.recipients = detail.summary.address.clone();
                        draft.subject = if detail.summary.subject.to_lowercase().starts_with("re:")
                        {
                            detail.summary.subject.clone()
                        } else {
                            format!("Re: {}", detail.summary.subject)
                        };
                        draft.reply_id = detail.rfc_message_id.clone().unwrap_or_default();
                    }
                    self.archive()?.save_draft(&draft)?;
                    self.draft = Some(draft);
                }
            },
            "draft" => {
                if let Some(draft) = &mut self.draft {
                    match text("field").as_str() {
                        "recipients" => draft.recipients = text("value"),
                        "subject" => draft.subject = text("value"),
                        "body" => draft.body = text("value"),
                        _ => {},
                    }
                    let draft = draft.clone();
                    self.archive()?.save_draft(&draft)?;
                }
            },
            "queue" => {
                if self.demo {
                    bail!("Sending is disabled in the synthetic demo");
                }
                if self.job.is_some() {
                    bail!("Wait for the current network operation to finish");
                }
                let account = Account::load(&self.root)?.context("Connect an account first")?;
                let draft = self.draft.clone().context("Compose a message first")?;
                let (raw, recipients) = moonmail_network::prepare_message(&account, &draft)?;
                let blob = FileBlobs::new(&self.root).put(&raw)?;
                let id = self
                    .archive_mut()?
                    .queue(&draft, &account.email, &blob, &recipients)?;
                self.draft = None;
                self.launch(move |root, _| Ok(moonmail_network::send_queued(&root, id)?))?;
            },
            "send" => {
                let id = message
                    .get("id")
                    .and_then(Data::as_i64)
                    .context("Missing outbox id")?;
                self.launch(move |root, _| Ok(moonmail_network::send_queued(&root, id)?))?;
            },
            "sync" => self.launch(|root, sender| {
                Ok(moonmail_network::sync_account(&root, |status| {
                    let _ = sender.try_send(Event::Progress(status));
                })?)
            })?,
            "login" => {
                anyhow::ensure!(
                    Account::load(&self.root)?.is_none(),
                    "One account is supported. Disconnect the current account before connecting another."
                );
                let account = Account::fastmail(&text("email"))?;
                self.launch(move |root, _| {
                    Auth::new(Keychain, &root).authorize(&account.email)?;
                    account.save(&root)?;
                    Ok(())
                })?;
            },
            "disconnect" => self.launch(|root, _| {
                if let Some(account) = Account::load(&root)? {
                    Auth::new(Keychain, &root).disconnect(&account.email)?;
                    std::fs::remove_file(root.join("account.json"))?;
                }
                Ok(())
            })?,
            "reindex" => {
                self.archive()?.reset_index()?;
                self.status = "Rebuilding search in bounded batches…".into();
            },
            _ => {},
        }
        Ok(())
    }
}

impl Native for Mail {
    fn start(&mut self, host: &Handle) -> std::result::Result<(), String> {
        (|| -> Result<()> {
            let archive = Archive::open(&self.root)?;
            self.preferences = crate::fonts::Preferences::load(&self.root)?;
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
            self.publish(host)
        })()
        .map_err(|error| format!("{error:#}"))
    }
    fn receive(&mut self, host: &Handle, _from: &str, message: Data) {
        self.error.clear();
        if let Err(error) = self.handle(message) {
            self.error = format!("{error:#}");
        }
        if let Err(error) = self.publish(host) {
            host.fail(format!("{error:#}"));
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
            while let Ok(event) = receiver.try_recv() {
                changed = true;
                match event {
                    Event::Progress(status) => self.status = status,
                    Event::Done(result) => {
                        done = true;
                        match result {
                            Ok(()) => self.status = "Up to date".into(),
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
        }
        if let (Some(archive), Some(index)) = (&self.archive, &mut self.index) {
            match crate::local::index_batch(archive, index) {
                Ok(indexed) => changed |= indexed,
                Err(error) => {
                    self.error = format!("{error:#}");
                    changed = true;
                },
            }
        }
        if changed && let Err(error) = self.refresh().and_then(|()| self.publish(host)) {
            host.fail(format!("{error:#}"));
        }
    }
}

#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
