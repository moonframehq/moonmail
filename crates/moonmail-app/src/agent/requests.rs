//! Requests the open app carries out: showing something in its window,
//! telling what the window shows, or doing what a click there does. The
//! agent's process records the request and waits; the app acts and answers
//! (see src/provider_outside.rs).

use {
    super::args::Args,
    crate::provider::AGENT_ACTIONS,
    anyhow::{Context, Result, bail},
    moonmail_core::MailRepository,
    moonmail_store::{AgentRequest, Archive},
    serde_json::{Value, json},
    std::{
        path::Path,
        time::{Duration, Instant},
    },
};

/// How long a request waits for the app, which answers within a tick.
const WAIT: Duration = Duration::from_secs(10);
/// How long a send waits for the user by default, and at most.
const SEND_WAIT: u64 = 300;
const MAX_WAIT: u64 = 600;

pub fn call(root: &Path, source: &str, name: &str, args: &Args<'_>) -> Result<(Value, String)> {
    let archive = Archive::open(root)?;
    let (kind, target, summary) = match name {
        "open_draft" => {
            let id = args.required("id")?;
            let draft = archive.draft(id)?.context("No draft with that ID")?;
            (
                "open_draft",
                id.to_owned(),
                format!("Opened draft “{}”", draft.subject),
            )
        },
        "show_message" => {
            let id = args.message("id")?;
            let subject = archive.detail(id)?.summary.subject;
            (
                "show_message",
                id.0.to_string(),
                format!("Showed “{subject}”"),
            )
        },
        "show_search" => {
            let query = args.required("query")?;
            moonmail_search::parse_query(query)?;
            (
                "show_search",
                query.to_owned(),
                format!("Showed search {query}"),
            )
        },
        "sync" => ("sync", String::new(), "Started sync".to_owned()),
        "app_state" => ("state", String::new(), "Read the window's state".to_owned()),
        "send_draft" => return send(&archive, source, args.required("id")?, args),
        "app_action" if args.text("action") == Some("queue") => {
            let id = match args.0.get("args").and_then(|fields| fields.get("id")) {
                Some(Value::String(id)) => id.clone(),
                _ => bail!("`queue` needs args.id, the draft to send"),
            };
            return send(&archive, source, &id, args);
        },
        "app_action" => {
            let action = args.required("action")?;
            if !AGENT_ACTIONS.contains(&action) {
                bail!(
                    "Only the user can do `{action}` in Moonmail (send, files, accounts, settings); send_draft asks them to send"
                );
            }
            let mut message = match args.0.get("args") {
                None | Some(Value::Null) => serde_json::Map::new(),
                Some(Value::Object(fields)) => fields.clone(),
                Some(_) => bail!("`args` must be an object"),
            };
            message.insert("action".into(), json!(action));
            let shown: String = Value::Object(message.clone())
                .to_string()
                .chars()
                .take(200)
                .collect();
            (
                "action",
                Value::Object(message).to_string(),
                format!("UI action {shown}"),
            )
        },
        _ => bail!("Unknown tool `{name}`"),
    };
    let id = archive.add_request(source, kind, &target, &summary)?;
    let request = wait_for(&archive, id, WAIT)?;
    let mut value = outcome(&request);
    if request.state == "done" && !request.detail.is_empty() {
        // The app's answer: the window's state, or what an action left.
        value["app"] = serde_json::from_str(&request.detail).unwrap_or(json!(request.detail));
        if let Some(fields) = value.as_object_mut() {
            fields.remove("detail");
        }
    }
    match request.state.as_str() {
        "done" => Ok((value, summary)),
        "pending" => {
            // Nobody acted on it: do not leave it for a later window.
            archive.settle_request(id, "expired", "Moonmail did not answer")?;
            bail!("Moonmail is not open, or did not answer; open it and try again")
        },
        _ => bail!("{}", request.detail),
    }
}

/// Ask the user to send a draft: Moonmail shows it in the assistant panel
/// and sends only on their click. Waits for the answer, leaving the request
/// pending (for `request_status`) when they have not answered yet.
fn send(archive: &Archive, source: &str, id: &str, args: &Args<'_>) -> Result<(Value, String)> {
    let draft = archive.draft(id)?.context("No draft with that ID")?;
    anyhow::ensure!(
        !draft.recipients.trim().is_empty(),
        "The draft has no recipients"
    );
    anyhow::ensure!(
        !draft.account.is_empty(),
        "The draft has no sending account"
    );
    let summary = format!("Asked to send “{}” to {}", draft.subject, draft.recipients);
    let request = archive.add_request(source, "send", id, &summary)?;
    let wait = args
        .integer("wait")?
        .map_or(SEND_WAIT, |wait| wait.clamp(0, MAX_WAIT as i64) as u64);
    let answer = wait_for(archive, request, Duration::from_secs(wait))?;
    let mut value = outcome(&answer);
    value["note"] = json!(match answer.state.as_str() {
        "pending" =>
            "The user has not answered yet; tell them it waits in the assistant panel, and check later with request_status",
        "done" => "The user approved; Moonmail is sending it",
        "denied" => "The user declined. Do not ask again unless they tell you to",
        "expired" => "This request no longer applies; ask again if still wanted",
        _ => "",
    });
    match answer.state.as_str() {
        "done" | "pending" | "denied" => Ok((value, summary)),
        _ => bail!("{}", answer.detail),
    }
}

/// `request_status`: an earlier request, after waiting while it is pending.
pub fn status(archive: &Archive, args: &Args<'_>) -> Result<Value> {
    let id = args.required_integer("id")?;
    archive
        .agent_request(id)?
        .context("No request with that ID")?;
    let wait = args
        .integer("wait")?
        .map_or(0, |wait| wait.clamp(0, MAX_WAIT as i64) as u64);
    Ok(outcome(&wait_for(archive, id, Duration::from_secs(wait))?))
}

/// Polls until the request leaves `pending` or `wait` passes.
fn wait_for(archive: &Archive, id: i64, wait: Duration) -> Result<AgentRequest> {
    let deadline = Instant::now() + wait;
    loop {
        let request = archive
            .agent_request(id)?
            .context("The request disappeared")?;
        if request.state != "pending" || Instant::now() >= deadline {
            return Ok(request);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn outcome(request: &AgentRequest) -> Value {
    json!({ "request": request.id, "kind": request.kind, "state": request.state, "detail": request.detail })
}

/// A request made as part of another tool, without waiting.
pub fn queue(
    archive: &Archive,
    source: &str,
    kind: &str,
    target: &str,
    summary: &str,
) -> Result<i64> {
    Ok(archive.add_request(source, kind, target, summary)?)
}
