use {
    moonmail_core::{FolderChange, MailQuery, MailRepository, MailboxRole},
    moonmail_network::{
        Account, FolderOutcome, IncomingMail, RemoteFolder, RemoteMessage, RemoteSnapshot,
        SyncError, synchronize, synchronize_cancellable,
    },
    moonmail_store::{Archive, FileBlobs, FolderEdit},
    std::{cell::Cell, collections::BTreeMap, io, rc::Rc},
};
type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;

struct Server {
    folders: BTreeMap<String, (u32, Vec<u32>)>,
    edits: usize,
    fail_list: bool,
    fail_after_edit: bool,
    lost_ack: bool,
    supported: bool,
    cancel: Option<Rc<Cell<bool>>>,
}
impl Server {
    fn new() -> Self {
        Self {
            folders: BTreeMap::from([
                ("INBOX".into(), (7, vec![])),
                ("Projects".into(), (8, vec![])),
                ("Projects/Review".into(), (9, vec![1])),
            ]),
            edits: 0,
            fail_list: false,
            fail_after_edit: false,
            lost_ack: false,
            supported: true,
            cancel: None,
        }
    }
}
impl IncomingMail for Server {
    type Error = io::Error;
    fn folders(&mut self) -> io::Result<Vec<RemoteFolder>> {
        if self.fail_list || (self.fail_after_edit && self.edits > 0) {
            return Err(io::Error::other("synthetic LIST failure"));
        }
        Ok(self
            .folders
            .keys()
            .map(|name| RemoteFolder {
                name: name.clone(),
                delimiter: Some("/".into()),
                selectable: true,
                role: if name == "INBOX" {
                    Some(MailboxRole::Inbox)
                } else {
                    None
                },
            })
            .collect())
    }
    fn snapshot(&mut self, name: &str) -> io::Result<RemoteSnapshot> {
        let (epoch, uids) = self
            .folders
            .get(name)
            .ok_or_else(|| io::Error::other("missing folder"))?;
        Ok(RemoteSnapshot {
            epoch: *epoch,
            messages: uids
                .iter()
                .map(|uid| RemoteMessage {
                    keywords: Vec::new(),
                    uid: *uid,
                    seen: true,
                    starred: false,
                    size: 100,
                    received_at: 100,
                })
                .collect(),
        })
    }
    fn fetch(&mut self, uid: u32) -> io::Result<Option<Vec<u8>>> {
        Ok(Some(format!("From: peer@example.org\r\nSubject: Folder fixture\r\nMessage-ID: <{uid}@example.org>\r\n\r\nRetained original").into_bytes()))
    }
    fn flag(&mut self, _: u32, _: &str, _: bool) -> io::Result<()> {
        Ok(())
    }
    fn edit_folder(&mut self, change: &FolderChange) -> io::Result<FolderOutcome> {
        if !self.supported {
            return Ok(FolderOutcome::Unsupported);
        }
        self.edits += 1;
        match change {
            FolderChange::Create { name } => {
                self.folders.insert(name.clone(), (10, vec![]));
            },
            FolderChange::Rename {
                source,
                destination,
            } => {
                let names: Vec<_> = self
                    .folders
                    .keys()
                    .filter(|name| *name == source || name.starts_with(&format!("{source}/")))
                    .cloned()
                    .collect();
                for name in names {
                    let value = self
                        .folders
                        .remove(&name)
                        .ok_or_else(|| io::Error::other("missing rename source"))?;
                    self.folders
                        .insert(format!("{destination}{}", &name[source.len()..]), value);
                }
            },
            FolderChange::Delete { name } => {
                self.folders.remove(name);
            },
        }
        if let Some(cancel) = &self.cancel {
            cancel.set(true);
        }
        if self.lost_ack {
            return Err(io::Error::other("mutation committed; response lost"));
        }
        Ok(FolderOutcome::Applied)
    }
}

#[test]
fn create_rename_hierarchy_and_delete_keep_ids_roles_flags_and_archived_bytes() -> Result {
    let dir = tempfile::tempdir()?;
    let mut archive = Archive::open(dir.path())?;
    let blobs = FileBlobs::new(dir.path());
    let account = Account::fastmail("me@example.org")?;
    let mut server = Server::new();
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    let parent = archive.mailbox(&account.email, "Projects")?;
    let child = archive.mailbox(&account.email, "Projects/Review")?;
    archive.set_mailbox_role(child, Some(MailboxRole::Archive))?;
    let message = archive.page(&MailQuery::default(), 10)?.rows[0].id;
    let blob = archive.detail(message)?.blob;
    archive.queue_folder_edit(&account.email, FolderEdit::Create("New folder".into()))?;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.name == "New folder" && folder.epoch == 10)
    );
    archive.queue_folder_edit(
        &account.email,
        FolderEdit::Rename {
            id: parent,
            name: "Work".into(),
        },
    )?;
    drop(archive);
    let mut archive = Archive::open(dir.path())?;
    archive.recover_interrupted_folders()?;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert!(
        archive
            .mailboxes()?
            .iter()
            .any(|folder| folder.id == child && folder.name == "Work/Review" && folder.total == 1)
    );
    assert_eq!(
        archive.role_mailbox(&account.email, MailboxRole::Archive)?,
        child
    );
    assert_eq!(archive.page(&MailQuery::default(), 10)?.rows[0].id, message);
    archive.queue_folder_edit(&account.email, FolderEdit::Delete(child))?;
    synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
    assert!(!server.folders.contains_key("Work/Review"));
    assert!(archive.page(&MailQuery::default(), 10)?.rows.is_empty());
    assert_eq!(archive.detail(message)?.blob, blob);
    assert_eq!(archive.detail(message)?.text, "Retained original");
    assert_eq!(server.edits, 3);
    assert!(
        archive
            .folder_operations()?
            .iter()
            .all(|operation| operation.state == "done")
    );
    Ok(())
}

#[test]
fn changed_namespace_epochs_unsupported_and_failed_list_never_dispatch_a_folder_mutation() -> Result
{
    for scenario in [
        "child",
        "epoch",
        "destination",
        "unsupported",
        "list",
        "cancel",
    ] {
        let dir = tempfile::tempdir()?;
        let mut archive = Archive::open(dir.path())?;
        let blobs = FileBlobs::new(dir.path());
        let account = Account::fastmail("me@example.org")?;
        let mut server = Server::new();
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        let parent = archive.mailbox(&account.email, "Projects")?;
        let operation = archive.queue_folder_edit(
            &account.email,
            FolderEdit::Rename {
                id: parent,
                name: "Work".into(),
            },
        )?;
        match scenario {
            "child" => {
                server.folders.insert("Projects/New".into(), (9, vec![]));
            },
            "epoch" => {
                server
                    .folders
                    .insert("Projects/Review".into(), (20, vec![1]));
            },
            "destination" => {
                server.folders.insert("Work".into(), (9, vec![]));
            },
            "unsupported" => server.supported = false,
            "list" => server.fail_list = true,
            "cancel" => archive.cancel_folder_edit(operation)?,
            _ => unreachable!(),
        }
        let result = synchronize(&mut server, &account, &mut archive, &blobs, |_| {});
        assert_eq!(result.is_err(), scenario == "list", "{scenario}");
        assert_eq!(server.edits, 0, "{scenario}");
        assert_eq!(
            archive.folder_operations()?[0].state,
            match scenario {
                "list" => "queued",
                "cancel" => "cancelled",
                _ => "failed",
            }
        );
    }
    Ok(())
}

#[test]
fn lost_responses_cancel_boundaries_and_failed_postmutation_list_never_repeat_namespace_edits()
-> Result {
    for scenario in [
        "lost_ack",
        "cancel_before",
        "cancel_after",
        "list_after",
        "crash",
    ] {
        let dir = tempfile::tempdir()?;
        let mut archive = Archive::open(dir.path())?;
        let blobs = FileBlobs::new(dir.path());
        let account = Account::fastmail("me@example.org")?;
        let mut server = Server::new();
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        let parent = archive.mailbox(&account.email, "Projects")?;
        let operation = archive.queue_folder_edit(
            &account.email,
            FolderEdit::Rename {
                id: parent,
                name: "Work".into(),
            },
        )?;
        let stop = Rc::new(Cell::new(false));
        server.lost_ack = scenario == "lost_ack";
        server.fail_after_edit = scenario == "list_after";
        if scenario == "cancel_after" {
            server.cancel = Some(stop.clone());
        }
        if scenario == "crash" {
            assert!(archive.claim_folder_edit(operation)?);
            archive.recover_interrupted_folders()?;
        }
        let result = synchronize_cancellable(
            &mut server,
            &account,
            &mut archive,
            &blobs,
            |status| {
                if scenario == "cancel_before" && status.starts_with("Renaming") {
                    stop.set(true);
                }
            },
            || stop.get(),
        );
        if scenario.starts_with("cancel") {
            assert!(matches!(result, Err(SyncError::Cancelled)));
        } else {
            assert_eq!(result.is_err(), scenario != "crash");
        }
        let expected = match scenario {
            "cancel_before" => "queued",
            "cancel_after" | "list_after" => "done",
            _ => "unknown",
        };
        assert_eq!(
            archive.folder_operations()?[0].state,
            expected,
            "{scenario}"
        );
        if scenario == "cancel_before" {
            archive.cancel_folder_edit(operation)?;
        }
        drop(archive);
        let mut archive = Archive::open(dir.path())?;
        archive.recover_interrupted_folders()?;
        server.lost_ack = false;
        server.fail_after_edit = false;
        server.cancel = None;
        stop.set(false);
        let edits = server.edits;
        synchronize(&mut server, &account, &mut archive, &blobs, |_| {})?;
        assert_eq!(server.edits, edits, "{scenario}");
        assert_eq!(
            archive.page(&MailQuery::default(), 10)?.rows.len(),
            1,
            "{scenario}"
        );
    }
    Ok(())
}
