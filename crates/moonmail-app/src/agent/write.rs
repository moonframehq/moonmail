use {
    super::{args::Args, read::page},
    anyhow::{Context, Result, bail},
    moonmail_core::{MailQuery, MailRepository, MailboxRole, MessageId},
    moonmail_network::{Account, Auth, Keychain},
    moonmail_store::{Archive, Draft, FileBlobs, KeywordRule, MoveTarget},
    serde_json::{Value, json},
    std::path::Path,
};

/// The most messages `rule_apply` touches in one call.
const RULE_APPLY_MAX: usize = 5000;

/// Runs a write and says what it did, for the activity log.
pub fn call(root: &Path, source: &str, name: &str, args: &Args<'_>) -> Result<(Value, String)> {
    let mut archive = Archive::open(root)?;
    Ok(match name {
        "set_flag" => {
            let ids = args.messages("ids")?;
            let flag = args.required("flag")?;
            let value = args.boolean("value")?.context("Missing `value`")?;
            let count = archive.set_flags(&ids, flag, value)?;
            let what = match (flag, value) {
                ("seen", true) => "read",
                ("seen", false) => "unread",
                (_, true) => "flagged",
                (_, false) => "unflagged",
            };
            (
                json!({ "updated": count }),
                format!("Marked {count} message(s) {what}"),
            )
        },
        "set_keyword" => {
            let ids = args.messages("ids")?;
            let keyword = args.required("keyword")?;
            let value = args.boolean("value")?.unwrap_or(true);
            let count = archive.set_keywords(&ids, keyword, value)?;
            let verb = if value { "Tagged" } else { "Untagged" };
            (
                json!({ "updated": count }),
                format!("{verb} {count} message(s) `{keyword}`"),
            )
        },
        "move" => {
            let ids = args.messages("ids")?;
            let to = args.token("to").context("Missing `to`")?;
            let count = archive.queue_moves(&ids, args.integer("from")?, target(&to)?)?;
            (
                json!({ "moved": count, "note": "Moved locally; the server follows when the app syncs" }),
                format!("Moved {count} message(s) to {to}"),
            )
        },
        "rule_save" => {
            let mailbox = args.required_integer("mailbox")?;
            let folder = archive
                .mailboxes()?
                .into_iter()
                .find(|folder| folder.id == mailbox)
                .context("No folder with that ID")?;
            let source = args.text("query").unwrap_or_default();
            let predicate = if source.trim().is_empty() {
                moonmail_core::MailPredicate::Account(folder.account)
            } else {
                moonmail_search::parse_query(source)?
            };
            let rule = KeywordRule {
                id: args.integer("id")?.unwrap_or(0),
                revision: args.integer("revision")?.unwrap_or(0),
                mailbox,
                name: args.required("name")?.trim().into(),
                source: source.into(),
                predicate,
                keyword: args.required("keyword")?.into(),
                value: args.boolean("value")?.unwrap_or(true),
                enabled: args.boolean("enabled")?.unwrap_or(true),
            };
            let id = archive.save_keyword_rule(&rule)?;
            (
                json!({ "id": id, "note": "Applies to messages entering the folder from now on; rule_apply runs it on current ones" }),
                format!("Saved rule “{}” on {}", rule.name, folder.name),
            )
        },
        "rule_delete" => {
            let id = args.required_integer("id")?;
            archive.delete_keyword_rule(id, args.required_integer("revision")?)?;
            (json!({ "deleted": id }), format!("Deleted rule {id}"))
        },
        "rule_apply" => {
            let id = args.required_integer("id")?;
            let rule = archive
                .keyword_rules()?
                .into_iter()
                .find(|rule| rule.id == id)
                .context("No rule with that ID")?;
            let ids = folder_matches(root, &mut archive, &rule)?;
            let mut applied = 0;
            for chunk in ids.chunks(100) {
                applied += archive.apply_keyword_rule(rule.id, rule.revision, chunk)?;
            }
            (
                json!({ "matched": ids.len(), "applied": applied, "capped": ids.len() == RULE_APPLY_MAX }),
                format!("Ran rule “{}” on {applied} message(s)", rule.name),
            )
        },
        "save_search" => {
            let name = args.required("name")?.trim();
            let source = args.required("query")?;
            let predicate = moonmail_search::parse_query(source)?;
            let id = archive.save_search(name, source, &predicate)?;
            (json!({ "id": id }), format!("Saved search “{name}”"))
        },
        "delete_search" => {
            let id = args.required_integer("id")?;
            let name = archive.saved_search(id)?.name;
            archive.delete_search(id)?;
            (
                json!({ "deleted": id }),
                format!("Deleted saved search “{name}”"),
            )
        },
        "create_draft" => {
            let draft = draft(root, &archive, args)?;
            archive.save_draft(&draft)?;
            let opened = if args.boolean("open")?.unwrap_or(true) {
                let summary = format!("Opened draft “{}”", draft.subject);
                Some(super::requests::queue(
                    &archive,
                    source,
                    "open_draft",
                    &draft.id,
                    &summary,
                )?)
            } else {
                None
            };
            (
                json!({ "id": draft.id, "to": draft.recipients, "subject": draft.subject,
                    "opened": opened.is_some(),
                    "note": "Saved as a draft; send_draft sends it" }),
                format!("Drafted “{}”", draft.subject),
            )
        },
        "update_draft" => {
            let id = args.required("id")?;
            let mut draft = archive.draft(id)?.context("No draft with that ID")?;
            for (key, field) in [
                ("to", &mut draft.recipients),
                ("cc", &mut draft.cc),
                ("bcc", &mut draft.bcc),
                ("subject", &mut draft.subject),
                ("body", &mut draft.body),
            ] {
                if let Some(value) = args.text(key) {
                    *field = value.into();
                }
            }
            if let Some(more) = args.text("append") {
                if !draft.body.is_empty() && !draft.body.ends_with('\n') {
                    draft.body.push('\n');
                }
                draft.body.push_str(more);
            }
            archive.save_draft(&draft)?;
            // A send waiting for the user was for the draft as it was.
            let cancelled = archive.expire_requests(
                "send",
                id,
                "The assistant changed the draft afterwards",
            )?;
            (
                json!({ "id": id, "cancelled_send_requests": cancelled }),
                format!("Edited draft “{}”", draft.subject),
            )
        },
        "delete_draft" => {
            let id = args.required("id")?;
            let draft = archive.draft(id)?.context("No draft with that ID")?;
            archive.discard_draft(id)?;
            archive.expire_requests("open_draft", id, "The draft was discarded")?;
            archive.expire_requests("send", id, "The draft was discarded")?;
            (
                json!({ "deleted": id }),
                format!("Discarded draft “{}”", draft.subject),
            )
        },
        "delete_folder" => {
            let id = args.required_integer("id")?;
            let folder = archive
                .mailboxes()?
                .into_iter()
                .find(|folder| folder.id == id)
                .context("No folder with that ID")?;
            folder_edit(
                root,
                &mut archive,
                &folder.account,
                moonmail_store::FolderEdit::Delete(id),
            )?;
            (
                json!({ "deleted": id, "note": "Local folders go at once; server folders on the next sync" }),
                format!("Deleted folder {} ({} messages)", folder.name, folder.total),
            )
        },
        "copy" => {
            let ids = args.messages("ids")?;
            let to = args.token("to").context("Missing `to`")?;
            let count = archive.queue_copies(&ids, args.integer("from")?, target(&to)?)?;
            (
                json!({ "copied": count }),
                format!("Copied {count} message(s) to {to}"),
            )
        },
        "mark_thread" => {
            let read = args.boolean("read")?.context("Missing `read`")?;
            let count = archive.set_thread_seen(args.message("id")?, read)?;
            let what = if read { "read" } else { "unread" };
            (
                json!({ "updated": count }),
                format!("Marked a thread of {count} {what}"),
            )
        },
        "create_folder" | "rename_folder" => {
            let name = args.required("name")?.to_owned();
            let (account, edit, summary) = if let Some(id) = args.integer("id")? {
                let folder = archive
                    .mailboxes()?
                    .into_iter()
                    .find(|folder| folder.id == id)
                    .context("No folder with that ID")?;
                let summary = format!("Renamed folder {} to {name}", folder.name);
                (
                    folder.account,
                    moonmail_store::FolderEdit::Rename { id, name },
                    summary,
                )
            } else {
                let summary = format!("Created folder {name}");
                (
                    args.required("account")?.to_owned(),
                    moonmail_store::FolderEdit::Create(name),
                    summary,
                )
            };
            folder_edit(root, &mut archive, &account, edit)?;
            (
                json!({ "account": account, "note": "Local folders change at once; server folders on the next sync" }),
                summary,
            )
        },
        "add_account" => {
            let email = args.required("email")?;
            let provider = args
                .text("provider")
                .or_else(|| {
                    email
                        .rsplit_once('@')
                        .and_then(|(_, domain)| crate::account_setup::domain_provider(domain))
                })
                .unwrap_or("fastmail");
            let account = Account::preset(crate::account_setup::provider(provider)?, email)?;
            account.check_registration(root)?;
            let auth = Auth::new(Keychain);
            auth.authorize_provider(account.provider, email)
                .context("Browser sign-in did not finish")?;
            moonmail_network::check_account(&account, &auth.credentials_for(&account)?)?;
            account.save(root)?;
            (
                json!({ "connected": email, "note": "Moonmail starts syncing it" }),
                format!("Connected {email}"),
            )
        },
        _ => bail!("Unknown tool `{name}`"),
    })
}

/// A folder ID, or a role such as `archive`.
fn target(to: &str) -> Result<MoveTarget> {
    Ok(match to.parse::<i64>() {
        Ok(id) => MoveTarget::Folder(id),
        Err(_) => MoveTarget::Role(
            MailboxRole::parse(&to.to_ascii_lowercase())
                .with_context(|| format!("No folder role `{to}`"))?,
        ),
    })
}

/// As the folder editor does it: local folders change now, server folders
/// queue for the account's worker.
pub fn folder_edit(
    root: &Path,
    archive: &mut Archive,
    account: &str,
    edit: moonmail_store::FolderEdit,
) -> Result<()> {
    let demo = std::env::var_os("MOONMAIL_DEMO").is_some();
    if demo || account == "local" {
        archive.edit_local_folder(account, edit)?;
    } else {
        anyhow::ensure!(
            Account::load_for(root, account)?.is_some(),
            "Connect this account before changing its server folders"
        );
        archive.queue_folder_edit(account, edit)?;
    }
    Ok(())
}

/// Messages now in the rule's folder that match it, up to the cap.
fn folder_matches(
    root: &Path,
    archive: &mut Archive,
    rule: &KeywordRule,
) -> Result<Vec<MessageId>> {
    let mut query = MailQuery {
        mailbox: Some(rule.mailbox),
        ..MailQuery::default()
    };
    let mut ids = Vec::new();
    loop {
        let found = page(root, archive, &rule.source, query.clone(), 100)?;
        ids.extend(found.rows.iter().map(|row| row.id));
        match found.next {
            Some(next) if ids.len() < RULE_APPLY_MAX => query.before = Some(next),
            _ => break,
        }
    }
    ids.truncate(RULE_APPLY_MAX);
    Ok(ids)
}

fn draft(root: &Path, archive: &Archive, args: &Args<'_>) -> Result<Draft> {
    let configured = match args.text("account") {
        Some(account) => Some(account.to_owned()),
        None => Account::load(root)?.map(|account| account.email),
    };
    let forward = args.integer("forward_of")?;
    anyhow::ensure!(
        forward.is_none() || args.integer("reply_to")?.is_none(),
        "Choose reply_to or forward_of, not both"
    );
    let mut draft = match args.integer("reply_to")?.or(forward) {
        Some(id) => {
            anyhow::ensure!(id > 0, "`reply_to` must be a positive message ID");
            let id = MessageId(id as u64);
            let detail = archive.detail(id)?;
            let headers = archive.reply_headers(
                id,
                &moonmail_mime::StalwartParser::default(),
                &FileBlobs::new(root),
            )?;
            let original = archive.message_account(id)?;
            let account = match configured {
                Some(account) if args.text("account").is_some() => account,
                _ if moonmail_network::validate_address(&original).is_ok() => original,
                configured => configured.context("Connect an account first")?,
            };
            let identities = archive.identities(&account)?;
            let identity = moonmail_network::choose_identity(&identities, &account, Some(&headers));
            if forward.is_some() {
                let mut draft = moonmail_network::compose_forward(&detail, &headers, &account)?;
                draft.identity = identity;
                crate::forward::copy_attachments(root, archive, &mut draft, id)?;
                draft
            } else {
                let mode = if args.boolean("reply_all")? == Some(true) {
                    moonmail_network::ReplyMode::All
                } else {
                    moonmail_network::ReplyMode::Sender
                };
                let mut draft = moonmail_network::compose_reply_with_identities(
                    &detail,
                    headers,
                    &account,
                    mode,
                    &identities,
                )?;
                draft.identity = identity;
                draft
            }
        },
        None => {
            let account = configured.context("Connect an account first")?;
            Draft {
                identity: moonmail_network::choose_identity(
                    &archive.identities(&account)?,
                    &account,
                    None,
                ),
                account,
                ..Default::default()
            }
        },
    };
    moonmail_network::insert_signature(&mut draft);
    if let Some(to) = args.text("to") {
        draft.recipients = to.into();
    }
    if let Some(cc) = args.text("cc") {
        draft.cc = cc.into();
    }
    if let Some(bcc) = args.text("bcc") {
        draft.bcc = bcc.into();
    }
    if let Some(subject) = args.text("subject") {
        draft.subject = subject.into();
    }
    let body = args.required("body")?;
    draft.body = if draft.body.is_empty() {
        body.into()
    } else {
        format!("{body}\n{}", draft.body)
    };
    draft.id = uuid::Uuid::new_v4().to_string();
    Ok(draft)
}
