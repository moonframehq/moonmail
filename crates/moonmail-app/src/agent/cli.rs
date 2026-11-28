use {
    super::tools::{Kind, TOOLS, Tool, call, find},
    anyhow::{Context, Result, bail},
    serde_json::{Map, Value, json},
    std::{path::Path, process::ExitCode},
};

/// `moonmail agent [TOOL [--param value ...]]`: one JSON document on stdout,
/// `{"error": ...}` and a failing exit status when the tool fails.
pub fn run(root: &Path, args: &[String]) -> ExitCode {
    let Some(name) = args.first().map(String::as_str) else {
        println!("{}", help());
        return ExitCode::SUCCESS;
    };
    let result = match name {
        "help" | "--help" | "-h" => {
            println!("{}", help());
            return ExitCode::SUCCESS;
        },
        "setup" => super::workspace::prepare(root, std::env::var_os("MOONMAIL_DEMO").is_some())
            .map(|dir| {
                let command = dir.join("moonmail").display().to_string();
                json!({
                    "dir": dir.display().to_string(),
                    "command": command,
                    "mcp": { "command": command, "args": ["mcp"] },
                    "note": "Run an agent in `dir` (its configs name the moonmail MCP server), or add the server to any MCP client",
                })
            }),
        "tools" => Ok(json!({ "tools": TOOLS.iter().map(|tool| json!({
            "name": tool.name, "description": tool.about, "writes": tool.writes(), "parameters": tool.schema(),
        })).collect::<Vec<_>>() })),
        _ => find(name)
            .context("Unknown tool; run `moonmail agent tools`")
            .and_then(|tool| parse(tool, &args[1..]))
            .and_then(|parsed| call(root, "cli", name, &parsed)),
    };
    match result {
        Ok(value) => {
            println!(
                "{}",
                serde_json::to_string_pretty(&value).unwrap_or_default()
            );
            ExitCode::SUCCESS
        },
        Err(error) => {
            println!("{}", json!({ "error": format!("{error:#}") }));
            ExitCode::FAILURE
        },
    }
}

/// `--name value`, `--name=value`, or a bare `--name` for true.
pub fn parse(tool: &Tool, args: &[String]) -> Result<Map<String, Value>> {
    let mut parsed = Map::new();
    let mut rest = args.iter().peekable();
    while let Some(arg) = rest.next() {
        let Some(flag) = arg.strip_prefix("--") else {
            bail!("Unexpected `{arg}`; parameters are `--name value`");
        };
        let (key, inline) = match flag.split_once('=') {
            Some((key, value)) => (key, Some(value.to_owned())),
            None => (flag, None),
        };
        // `--reply-to` and `--reply_to` both name `reply_to`.
        let key = &key.replace('-', "_");
        let param = tool
            .params
            .iter()
            .find(|param| param.name == key)
            .with_context(|| format!("`{}` takes no `--{key}`", tool.name))?;
        let value = match inline {
            Some(value) => Some(value),
            None if param.kind == Kind::Boolean
                && rest.peek().is_none_or(|next| next.starts_with("--")) =>
            {
                None
            },
            None => Some(
                rest.next()
                    .with_context(|| format!("`--{key}` needs a value"))?
                    .clone(),
            ),
        };
        let value = match (param.kind, value) {
            (Kind::Boolean, None) => Value::Bool(true),
            (Kind::Boolean, Some(value)) => match value.as_str() {
                "true" | "yes" | "1" => Value::Bool(true),
                "false" | "no" | "0" => Value::Bool(false),
                _ => bail!("`--{key}` is true or false"),
            },
            (_, None) => bail!("`--{key}` needs a value"),
            (Kind::Integer, Some(value)) => Value::from(
                value
                    .parse::<i64>()
                    .with_context(|| format!("`--{key}` must be an integer"))?,
            ),
            (Kind::Ids, Some(value)) => Value::Array(
                value
                    .split(',')
                    .map(|id| {
                        id.trim()
                            .parse::<i64>()
                            .map(Value::from)
                            .with_context(|| format!("`--{key}` takes IDs such as 4,8,15"))
                    })
                    .collect::<Result<_>>()?,
            ),
            (Kind::Folder, Some(value)) => value
                .parse::<i64>()
                .map_or_else(|_| Value::String(value), Value::from),
            (Kind::Text, Some(value)) => Value::String(value),
            (Kind::Object, Some(value)) => match serde_json::from_str(&value) {
                Ok(Value::Object(object)) => Value::Object(object),
                _ => bail!("`--{key}` takes a JSON object"),
            },
        };
        parsed.insert(key.into(), value);
    }
    if let Some(missing) = tool
        .params
        .iter()
        .find(|param| param.required && !parsed.contains_key(param.name))
    {
        bail!("`{}` needs `--{}`", tool.name, missing.name);
    }
    Ok(parsed)
}

fn help() -> String {
    let mut text = String::from(
        "moonmail agent TOOL [--param value ...]\n\nJSON on stdout. Works while Moonmail is open: the app shows each change as it lands.\nEvery call is logged (see `activity`); tools marked * change mail or the app, so agents ask first. `moonmail agent tools` prints JSON Schemas.\n`moonmail mcp` serves the same tools over MCP (stdio). `moonmail agent setup` writes an agent folder with each agent's MCP configuration.\n\n",
    );
    for tool in TOOLS {
        text.push_str(&format!(
            "  {}{}  {}\n",
            tool.name,
            if tool.writes() { "*" } else { "" },
            tool.about
        ));
        for param in tool.params {
            text.push_str(&format!(
                "      --{}{}  {}\n",
                param.name,
                if param.required { "" } else { " (optional)" },
                param.about
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn flags_follow_each_parameter_kind() -> Result<()> {
        let tool = find("move").context("move")?;
        let parsed = parse(tool, &strings(&["--ids", "4,8", "--to", "archive"]))?;
        assert_eq!(parsed["ids"], json!([4, 8]));
        assert_eq!(parsed["to"], json!("archive"));
        let parsed = parse(tool, &strings(&["--ids=15", "--to=12", "--from", "3"]))?;
        assert_eq!(parsed["from"], json!(3));
        assert_eq!(parsed["to"], json!(12));
        assert!(parse(tool, &strings(&["--to", "archive"])).is_err());
        assert!(parse(tool, &strings(&["--ids", "x", "--to", "1"])).is_err());
        assert!(parse(tool, &strings(&["--nope", "1"])).is_err());

        let search = find("search").context("search")?;
        let parsed = parse(search, &strings(&["--unread", "--query", "from:alice"]))?;
        assert_eq!(parsed["unread"], json!(true));
        assert_eq!(parsed["query"], json!("from:alice"));
        let parsed = parse(search, &strings(&["--unread", "false"]))?;
        assert_eq!(parsed["unread"], json!(false));
        Ok(())
    }
}
