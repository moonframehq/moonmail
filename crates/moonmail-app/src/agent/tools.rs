use {
    super::{args::Args, read, requests, write},
    anyhow::{Result, bail},
    serde_json::{Map, Value, json},
    std::path::Path,
};

/// What a parameter holds, for the JSON Schema and command-line parsing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Text,
    Integer,
    Boolean,
    /// Message IDs: a JSON list, or `1,2,3` on the command line.
    Ids,
    /// A folder ID or a role name such as `archive`.
    Folder,
    /// A JSON object; `--name '{"key":1}'` on the command line.
    Object,
}

/// What a tool does, which decides how it runs and whether agents ask first.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Only looks at mail.
    Read,
    /// Changes mail (and, through the outbox, the server).
    Write,
    /// Asks the open app to show something or tell what it shows; changes
    /// no mail.
    Show,
    /// Asks the open app to act as a click would.
    Act,
}

impl Tool {
    /// May change mail or the app: agents ask the user first.
    pub fn writes(&self) -> bool {
        matches!(self.effect, Effect::Write | Effect::Act)
    }
}

pub struct Param {
    pub name: &'static str,
    pub kind: Kind,
    pub required: bool,
    pub about: &'static str,
}

pub struct Tool {
    pub name: &'static str,
    pub about: &'static str,
    pub effect: Effect,
    pub params: &'static [Param],
}

const fn p(name: &'static str, kind: Kind, required: bool, about: &'static str) -> Param {
    Param {
        name,
        kind,
        required,
        about,
    }
}

const QUERY: &str = "Moonmail search: words, \"phrases\", subject: body: from: to: cc: account: in:FOLDER is:unread|read|starred keyword: tag: sender-address: recipient-address: after:YYYY-MM-DD before:YYYY-MM-DD, AND/OR/NOT and parentheses";
const APP_ACTION: &str = "Do what a click in Moonmail's window does, by its UI action: navigation (folder, account, filter, search, select, next, previous, sort, threaded, layout), selection (toggle_selection, selection_flag, star, unread, thread_read), filing (archive_messages, trash_messages, move_messages, copy_messages, keyword_set), writing (compose, reply, reply_all, forward, draft with field recipients|cc|bcc|subject|body and value, draft_open, draft_discard, queue_only), queue (send the open draft: asks the user like send_draft), rules (rule_save, rule_apply, rule_delete), folders (folder_create, folder_rename, folder_delete, folder_role, folder_sync) and sync. Actions that open files, change accounts, credentials or settings, delete permanently, or send are the user's own. Check the result with app_state.";
const LIMIT: Param = p("limit", Kind::Integer, false, "Most rows returned");
const ID: Param = p("id", Kind::Integer, true, "Local message ID");
const IDS: Param = p("ids", Kind::Ids, true, "Local message IDs, at most 100");

pub const TOOLS: &[Tool] = &[
    Tool {
        name: "accounts",
        about: "List connected mail accounts.",
        effect: Effect::Read,
        params: &[],
    },
    Tool {
        name: "mailboxes",
        about: "List folders with their IDs, roles, and message and unread counts.",
        effect: Effect::Read,
        params: &[p("account", Kind::Text, false, "Only this account")],
    },
    Tool {
        name: "search",
        about: "Find messages, newest first: metadata and a snippet, never bodies. Without a query, lists a folder or account.",
        effect: Effect::Read,
        params: &[
            p("query", Kind::Text, false, QUERY),
            p("mailbox", Kind::Integer, false, "Only this folder ID"),
            p("account", Kind::Text, false, "Only this account"),
            p("unread", Kind::Boolean, false, "Only unread messages"),
            LIMIT,
        ],
    },
    Tool {
        name: "message",
        about: "Read one message: headers, plain-text body, keywords and folders.",
        effect: Effect::Read,
        params: &[
            ID,
            p(
                "max_chars",
                Kind::Integer,
                false,
                "Body characters returned (default 20000)",
            ),
        ],
    },
    Tool {
        name: "thread",
        about: "Messages in the same reply thread as a message.",
        effect: Effect::Read,
        params: &[ID, LIMIT],
    },
    Tool {
        name: "correspondence",
        about: "Messages exchanged with the same people as a message, in either direction.",
        effect: Effect::Read,
        params: &[ID, LIMIT],
    },
    Tool {
        name: "rules",
        about: "List folder-entry rules: each sets or clears a keyword on messages entering its folder.",
        effect: Effect::Read,
        params: &[],
    },
    Tool {
        name: "rule_preview",
        about: "Dry run: which messages now in a folder match a rule condition. Changes nothing.",
        effect: Effect::Read,
        params: &[
            p("mailbox", Kind::Integer, true, "Folder ID the rule watches"),
            p(
                "query",
                Kind::Text,
                false,
                "Condition using metadata fields only; empty matches all",
            ),
            LIMIT,
        ],
    },
    Tool {
        name: "saved_searches",
        about: "List saved searches.",
        effect: Effect::Read,
        params: &[],
    },
    Tool {
        name: "drafts",
        about: "List drafts, newest first.",
        effect: Effect::Read,
        params: &[LIMIT],
    },
    Tool {
        name: "activity",
        about: "Changes made through this interface, oldest first.",
        effect: Effect::Read,
        params: &[
            p("after", Kind::Integer, false, "Only entries after this ID"),
            LIMIT,
        ],
    },
    Tool {
        name: "set_flag",
        about: "Mark messages read or unread, flagged or unflagged.",
        effect: Effect::Write,
        params: &[
            IDS,
            p("flag", Kind::Text, true, "`seen` or `flagged`"),
            p(
                "value",
                Kind::Boolean,
                true,
                "true sets the flag, false clears it",
            ),
        ],
    },
    Tool {
        name: "set_keyword",
        about: "Add or remove a keyword (tag) on messages.",
        effect: Effect::Write,
        params: &[
            IDS,
            p(
                "keyword",
                Kind::Text,
                true,
                "IMAP keyword, such as `followup`",
            ),
            p(
                "value",
                Kind::Boolean,
                false,
                "false removes it (default true)",
            ),
        ],
    },
    Tool {
        name: "move",
        about: "Move messages to a folder or role; the server follows when the app syncs.",
        effect: Effect::Write,
        params: &[
            IDS,
            p(
                "to",
                Kind::Folder,
                true,
                "Folder ID, or a role: archive, trash, inbox, junk",
            ),
            p(
                "from",
                Kind::Integer,
                false,
                "Source folder ID; default all of the message's folders",
            ),
        ],
    },
    Tool {
        name: "rule_save",
        about: "Create a folder-entry rule, or replace one by id and revision. Applies to future arrivals only; use rule_apply for current messages.",
        effect: Effect::Write,
        params: &[
            p("mailbox", Kind::Integer, true, "Folder ID the rule watches"),
            p("name", Kind::Text, true, "Rule name, unique in its folder"),
            p(
                "query",
                Kind::Text,
                false,
                "Condition using metadata fields only; empty matches all",
            ),
            p(
                "keyword",
                Kind::Text,
                true,
                "Keyword the rule sets or clears",
            ),
            p(
                "value",
                Kind::Boolean,
                false,
                "false clears the keyword (default true)",
            ),
            p("enabled", Kind::Boolean, false, "Default true"),
            p("id", Kind::Integer, false, "Rule to replace"),
            p("revision", Kind::Integer, false, "Its current revision"),
        ],
    },
    Tool {
        name: "rule_delete",
        about: "Delete a folder-entry rule.",
        effect: Effect::Write,
        params: &[
            p("id", Kind::Integer, true, "Rule ID"),
            p("revision", Kind::Integer, true, "Its current revision"),
        ],
    },
    Tool {
        name: "rule_apply",
        about: "Run a rule now on the messages already in its folder (at most 5000).",
        effect: Effect::Write,
        params: &[p("id", Kind::Integer, true, "Rule ID")],
    },
    Tool {
        name: "save_search",
        about: "Save a search by name; an existing name is replaced.",
        effect: Effect::Write,
        params: &[
            p("name", Kind::Text, true, "Name shown in the sidebar"),
            p("query", Kind::Text, true, QUERY),
        ],
    },
    Tool {
        name: "delete_search",
        about: "Delete a saved search.",
        effect: Effect::Write,
        params: &[p("id", Kind::Integer, true, "Saved search ID")],
    },
    Tool {
        name: "create_draft",
        about: "Save a draft for the user to review and send. Never sends.",
        effect: Effect::Write,
        params: &[
            p(
                "to",
                Kind::Text,
                false,
                "Recipients, comma-separated; a reply fills them",
            ),
            p("cc", Kind::Text, false, "Cc recipients"),
            p("subject", Kind::Text, false, "Subject; a reply fills it"),
            p(
                "body",
                Kind::Text,
                true,
                "Plain-text body, above any quote and signature",
            ),
            p("reply_to", Kind::Integer, false, "Message ID to reply to"),
            p("reply_all", Kind::Boolean, false, "Reply to everyone"),
            p(
                "account",
                Kind::Text,
                false,
                "Sending account; default the first",
            ),
            p("bcc", Kind::Text, false, "Bcc recipients"),
            p(
                "forward_of",
                Kind::Integer,
                false,
                "Message ID to forward, with its attachments",
            ),
            p(
                "open",
                Kind::Boolean,
                false,
                "Open it in the composer window (default true)",
            ),
        ],
    },
    Tool {
        name: "update_draft",
        about: "Change a draft's fields; the open composer follows.",
        effect: Effect::Write,
        params: &[
            p("id", Kind::Text, true, "Draft ID"),
            p("to", Kind::Text, false, "Recipients"),
            p("cc", Kind::Text, false, "Cc recipients"),
            p("bcc", Kind::Text, false, "Bcc recipients"),
            p("subject", Kind::Text, false, "Subject"),
            p(
                "body",
                Kind::Text,
                false,
                "Whole new body, replacing the old one",
            ),
            p(
                "append",
                Kind::Text,
                false,
                "Text added at the end of the body",
            ),
        ],
    },
    Tool {
        name: "delete_draft",
        about: "Discard a draft.",
        effect: Effect::Write,
        params: &[p("id", Kind::Text, true, "Draft ID")],
    },
    Tool {
        name: "copy",
        about: "Copy messages to a folder; the originals stay.",
        effect: Effect::Write,
        params: &[
            IDS,
            p(
                "to",
                Kind::Folder,
                true,
                "Folder ID, or a role: archive, inbox",
            ),
            p("from", Kind::Integer, false, "Source folder ID"),
        ],
    },
    Tool {
        name: "mark_thread",
        about: "Mark a message's whole thread read or unread, across folders.",
        effect: Effect::Write,
        params: &[ID, p("read", Kind::Boolean, true, "true marks read")],
    },
    Tool {
        name: "create_folder",
        about: "Create a folder in an account.",
        effect: Effect::Write,
        params: &[
            p("account", Kind::Text, true, "Account email, or `local`"),
            p(
                "name",
                Kind::Text,
                true,
                "Full folder name, using the server's separator for nesting",
            ),
        ],
    },
    Tool {
        name: "rename_folder",
        about: "Rename a folder.",
        effect: Effect::Write,
        params: &[
            p("id", Kind::Integer, true, "Folder ID"),
            p("name", Kind::Text, true, "New full name"),
        ],
    },
    Tool {
        name: "open_draft",
        about: "Show a draft in Moonmail's composer window.",
        effect: Effect::Show,
        params: &[p("id", Kind::Text, true, "Draft ID")],
    },
    Tool {
        name: "show_message",
        about: "Select a message in the Moonmail window so the user sees it.",
        effect: Effect::Show,
        params: &[ID],
    },
    Tool {
        name: "show_search",
        about: "Run a search in the Moonmail window so the user sees the results.",
        effect: Effect::Show,
        params: &[p("query", Kind::Text, true, QUERY)],
    },
    Tool {
        name: "sync",
        about: "Ask Moonmail to sync every account now.",
        effect: Effect::Act,
        params: &[],
    },
    Tool {
        name: "send_draft",
        about: "Send a draft. Moonmail shows it to the user in the assistant panel with Send and Deny, and sends only when they press Send; this waits for their answer. If it is still pending, tell the user and check later with request_status.",
        effect: Effect::Act,
        params: &[
            p("id", Kind::Text, true, "Draft ID"),
            p(
                "wait",
                Kind::Integer,
                false,
                "Seconds to wait for the user's answer (default 300)",
            ),
        ],
    },
    Tool {
        name: "delete_folder",
        about: "Delete a folder and the messages in it.",
        effect: Effect::Write,
        params: &[p("id", Kind::Integer, true, "Folder ID")],
    },
    Tool {
        name: "app_state",
        about: "What the open Moonmail window shows: the view, folder, search, listed rows, selected and checked messages, the pinned Correspondence or Reply Thread anchor and current page of related messages, the open draft, status and errors. Use it to see the effect of app_action.",
        effect: Effect::Show,
        params: &[],
    },
    Tool {
        name: "app_action",
        about: APP_ACTION,
        effect: Effect::Act,
        params: &[
            p("action", Kind::Text, true, "The action's name"),
            p(
                "args",
                Kind::Object,
                false,
                "Its other fields, as the UI sends them",
            ),
            p(
                "wait",
                Kind::Integer,
                false,
                "For queue: seconds to wait for the user's answer (default 300)",
            ),
        ],
    },
    Tool {
        name: "request_status",
        about: "The state of an earlier request such as send_draft: pending (waiting for the user), done, denied, failed or expired.",
        effect: Effect::Read,
        params: &[
            p("id", Kind::Integer, true, "Request ID"),
            p(
                "wait",
                Kind::Integer,
                false,
                "Seconds to wait while it is pending (default 0)",
            ),
        ],
    },
    Tool {
        name: "draft",
        about: "Read one draft in full.",
        effect: Effect::Read,
        params: &[p("id", Kind::Text, true, "Draft ID")],
    },
    Tool {
        name: "outbox",
        about: "Messages queued or sent from Moonmail, newest first, with their delivery state.",
        effect: Effect::Read,
        params: &[LIMIT],
    },
    Tool {
        name: "add_account",
        about: "Connect an account with browser sign-in (OAuth). The user finishes it in their browser; passwords never pass through here.",
        effect: Effect::Write,
        params: &[
            p("email", Kind::Text, true, "Address to connect"),
            p(
                "provider",
                Kind::Text,
                false,
                "fastmail, google, microsoft; default from the domain",
            ),
        ],
    },
];

pub fn find(name: &str) -> Option<&'static Tool> {
    TOOLS.iter().find(|tool| tool.name == name)
}

impl Tool {
    /// The parameters as a JSON Schema object.
    pub fn schema(&self) -> Value {
        let mut properties = Map::new();
        for param in self.params {
            let kind = match param.kind {
                Kind::Text => json!({ "type": "string" }),
                Kind::Integer => json!({ "type": "integer" }),
                Kind::Boolean => json!({ "type": "boolean" }),
                Kind::Ids => {
                    json!({ "type": "array", "items": { "type": "integer" }, "maxItems": 100 })
                },
                Kind::Folder => json!({ "type": ["integer", "string"] }),
                Kind::Object => json!({ "type": "object" }),
            };
            let mut kind = kind;
            kind["description"] = json!(param.about);
            properties.insert(param.name.into(), kind);
        }
        let required: Vec<_> = self
            .params
            .iter()
            .filter(|param| param.required)
            .map(|param| param.name)
            .collect();
        json!({ "type": "object", "properties": properties, "required": required, "additionalProperties": false })
    }
}

/// Run a tool and log the call, whether it succeeds or not. `source` (`cli`
/// or `mcp`) names who called it.
pub fn call(root: &Path, source: &str, name: &str, args: &Map<String, Value>) -> Result<Value> {
    let Some(tool) = find(name) else {
        bail!("Unknown tool `{name}`; run `moonmail agent tools`");
    };
    let result = run(root, source, tool, args);
    let summary = match &result {
        Ok((_, summary)) => summary.clone(),
        Err(error) => format!("{error:#}"),
    };
    moonmail_store::Archive::open(root)?.record_activity(&moonmail_store::Call {
        source,
        tool: name,
        writes: tool.writes(),
        ok: result.is_ok(),
        summary: &summary,
    })?;
    result.map(|(value, _)| value)
}

fn run(
    root: &Path,
    source: &str,
    tool: &Tool,
    args: &Map<String, Value>,
) -> Result<(Value, String)> {
    if let Some(key) = args
        .keys()
        .find(|key| !tool.params.iter().any(|param| param.name == key.as_str()))
    {
        bail!("`{}` takes no `{key}`", tool.name);
    }
    let summary = || {
        let shown = Value::Object(args.clone()).to_string();
        let shown: String = shown.chars().take(200).collect();
        format!("{} {shown}", tool.name)
    };
    let args = Args(args);
    match tool.effect {
        Effect::Read => read::call(root, tool.name, &args).map(|value| (value, summary())),
        Effect::Show | Effect::Act => requests::call(root, source, tool.name, &args),
        Effect::Write => write::call(root, source, tool.name, &args),
    }
}
