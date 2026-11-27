use {
    super::args::Args,
    anyhow::{Context, Result, bail},
    moonmail_core::{MailPage, MailQuery, MailRepository, MessageId, ViewFilter},
    moonmail_network::Account,
    moonmail_search::SqliteIndex,
    moonmail_store::Archive,
    serde_json::{Value, json},
    std::path::Path,
};

pub fn call(root: &Path, name: &str, args: &Args<'_>) -> Result<Value> {
    let mut archive = Archive::open(root)?;
    Ok(match name {
        "accounts" => {
            let accounts: Vec<_> = Account::load_all(root)?
                .into_iter()
                .map(|account| {
                    json!({ "email": account.email, "provider": account.provider, "transport": account.transport })
                })
                .collect();
            json!({ "accounts": accounts })
        },
        "mailboxes" => {
            let account = args.text("account");
            let mailboxes: Vec<_> = archive
                .mailboxes()?
                .into_iter()
                .filter(|mailbox| account.is_none_or(|account| mailbox.account == account))
                .map(|mailbox| {
                    json!({ "id": mailbox.id, "account": mailbox.account, "name": mailbox.name,
                        "role": mailbox.role, "total": mailbox.total, "unread": mailbox.unread })
                })
                .collect();
            json!({ "mailboxes": mailboxes })
        },
        "search" => {
            let query = MailQuery {
                mailbox: args.integer("mailbox")?,
                account: args.text("account").map(str::to_owned),
                filter: if args.boolean("unread")? == Some(true) {
                    ViewFilter::Unread
                } else {
                    ViewFilter::All
                },
                ..MailQuery::default()
            };
            let page = page(
                root,
                &mut archive,
                args.text("query").unwrap_or_default(),
                query,
                args.limit(20, 100)?,
            )?;
            json!({ "messages": page.rows, "more": page.next.is_some() })
        },
        "message" => {
            let id = args.message("id")?;
            let detail = archive.detail(id)?;
            let max = args
                .integer("max_chars")?
                .map_or(20_000, |max| max.clamp(100, 200_000) as usize);
            let text: String = detail.text.chars().take(max).collect();
            let truncated = text.len() < detail.text.len();
            json!({
                "message": detail.summary,
                "account": archive.message_account(id)?,
                "keywords": archive.message_keywords(id)?,
                "folders": folders(&archive, id)?,
                "message_id_header": detail.rfc_message_id,
                "has_html": detail.html.is_some(),
                "text": text,
                "truncated": truncated,
            })
        },
        "thread" => {
            json!({ "messages": archive.thread(args.message("id")?, args.limit(50, 200)?)? })
        },
        "correspondence" => json!({
            "messages": archive.correspondence(args.message("id")?, args.limit(50, 200)?)?
        }),
        "rules" => json!({ "rules": archive.keyword_rules()? }),
        "rule_preview" => {
            let query = MailQuery {
                mailbox: Some(args.required_integer("mailbox")?),
                ..MailQuery::default()
            };
            let page = page(
                root,
                &mut archive,
                args.text("query").unwrap_or_default(),
                query,
                args.limit(20, 100)?,
            )?;
            json!({ "matches": page.rows, "more": page.next.is_some() })
        },
        "saved_searches" => json!({ "saved_searches": archive.saved_searches()? }),
        "drafts" => json!({ "drafts": archive.drafts(None, args.limit(20, 50)?)?.rows }),
        "request_status" => super::requests::status(&archive, args)?,
        "draft" => json!({
            "draft": archive.draft(args.required("id")?)?.context("No draft with that ID")?
        }),
        "outbox" => json!({ "outbox": archive.outbox_page(None, args.limit(20, 50)?)?.rows }),
        "activity" => json!({
            "activity": archive.activity_after(args.integer("after")?.unwrap_or(0), args.limit(50, 500)?)?
        }),
        _ => bail!("Unknown tool `{name}`"),
    })
}

/// The folders holding a message, by ID and name.
fn folders(archive: &Archive, id: MessageId) -> Result<Vec<Value>> {
    let account = archive.message_account(id)?;
    let mut found = Vec::new();
    for mailbox in archive.mailboxes()? {
        if mailbox.account != account {
            continue;
        }
        let query = MailQuery {
            mailbox: Some(mailbox.id),
            ids: Some(vec![id]),
            ..MailQuery::default()
        };
        if !archive.page(&query, 1)?.rows.is_empty() {
            found.push(json!({ "id": mailbox.id, "name": mailbox.name }));
        }
    }
    Ok(found)
}

/// One page of `query`, restricted to `source` when it is not empty. Reads the
/// search index as the app last built it; this never indexes.
pub fn page(
    root: &Path,
    archive: &mut Archive,
    source: &str,
    mut query: MailQuery,
    limit: usize,
) -> Result<MailPage> {
    if !source.trim().is_empty() {
        let predicate = moonmail_search::parse_query(source)?;
        let path = root.join("search.sqlite3");
        anyhow::ensure!(
            path.exists(),
            "The search index is not built yet; open Moonmail or run `moonmail reindex`"
        );
        let index = SqliteIndex::open(path).context("Could not open the search index")?;
        archive.prepare_predicate(&index, &predicate)?;
        query.search_matches = true;
    }
    Ok(archive.page(&query, limit)?)
}
