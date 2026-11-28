//! The folder the assistant's agent works in: a `moonmail` command bound to
//! this archive, the guide agents read on start (with every tool, from the
//! tool table), and each agent's project configuration naming Moonmail's MCP
//! server. Claude's also allows the read-only tools and asks before each
//! writing one. Rewritten at each launch so it follows the running binary.

use {
    super::tools::TOOLS,
    anyhow::Result,
    serde_json::json,
    std::path::{Path, PathBuf},
};

const GUIDE: &str = include_str!("guide.md");

pub fn prepare(root: &Path, demo: bool) -> Result<PathBuf> {
    let dir = root.join("agent");
    std::fs::create_dir_all(&dir)?;
    let exe = std::env::current_exe()?;
    let command = dir.join("moonmail");
    write_if_changed(
        &command,
        &format!(
            "#!/bin/sh\n# Moonmail bound to this archive; regenerated at each launch.\n{}MOONMAIL_DATA_DIR={} exec {} \"$@\"\n",
            if demo { "MOONMAIL_DEMO=1 " } else { "" },
            quote(&root.display().to_string()),
            quote(&exe.display().to_string()),
        ),
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&command, std::fs::Permissions::from_mode(0o700))?;
    }
    let guide = GUIDE.replace("{TOOLS}", &tool_list());
    for name in ["AGENTS.md", "CLAUDE.md"] {
        write_if_changed(&dir.join(name), &guide)?;
    }
    let program = command.display().to_string();
    let json = |value: serde_json::Value| serde_json::to_string_pretty(&value);
    // Claude Code: the project's MCP servers, enabled without a prompt.
    write_if_changed(
        &dir.join(".mcp.json"),
        &json(
            json!({ "mcpServers": { "moonmail": { "type": "stdio", "command": program, "args": ["mcp"] } } }),
        )?,
    )?;
    let names = |writes: bool| -> Vec<String> {
        TOOLS
            .iter()
            .filter(|tool| tool.writes() == writes)
            .map(|tool| format!("mcp__moonmail__{}", tool.name))
            .collect()
    };
    std::fs::create_dir_all(dir.join(".claude"))?;
    write_if_changed(
        &dir.join(".claude/settings.json"),
        &json(json!({
            "enableAllProjectMcpServers": true,
            "enabledMcpjsonServers": ["moonmail"],
            "permissions": { "allow": names(false), "ask": names(true) },
        }))?,
    )?;
    // Gemini CLI asks before each MCP call unless a server is trusted.
    std::fs::create_dir_all(dir.join(".gemini"))?;
    write_if_changed(
        &dir.join(".gemini/settings.json"),
        &json(json!({ "mcpServers": { "moonmail": { "command": program, "args": ["mcp"] } } }))?,
    )?;
    // OpenCode reads the folder's opencode.json.
    write_if_changed(
        &dir.join("opencode.json"),
        &json(json!({
            "$schema": "https://opencode.ai/config.json",
            "mcp": { "moonmail": { "type": "local", "command": [program, "mcp"], "enabled": true } },
        }))?,
    )?;
    // Codex reads a project's .codex/config.toml once the project is trusted.
    std::fs::create_dir_all(dir.join(".codex"))?;
    write_if_changed(
        &dir.join(".codex/config.toml"),
        &format!(
            "[mcp_servers.moonmail]\ncommand = {}\nargs = [\"mcp\"]\n",
            serde_json::to_string(&program)?
        ),
    )?;
    Ok(dir)
}

/// Every tool, for the guide.
fn tool_list() -> String {
    let mut text = String::new();
    for tool in TOOLS {
        text.push_str(&format!(
            "- `{}`{}: {}\n",
            tool.name,
            if tool.writes() { " (**writes**)" } else { "" },
            tool.about
        ));
        for param in tool.params {
            text.push_str(&format!(
                "  - `{}`{}: {}\n",
                param.name,
                if param.required { "" } else { " (optional)" },
                param.about
            ));
        }
    }
    text
}

fn write_if_changed(path: &Path, contents: &str) -> Result<()> {
    if std::fs::read_to_string(path).ok().as_deref() != Some(contents) {
        std::fs::write(path, contents)?;
    }
    Ok(())
}

/// A POSIX shell single-quoted word.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_binds_the_archive_and_quotes_paths() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().join("it's mail");
        let agent = prepare(&root, false)?;
        let script = std::fs::read_to_string(agent.join("moonmail"))?;
        assert!(script.contains(r"MOONMAIL_DATA_DIR='"));
        assert!(script.contains(r"it'\''s mail'"));
        assert!(
            std::fs::read_to_string(agent.join("CLAUDE.md"))?.contains("./moonmail agent TOOL")
        );
        let guide = std::fs::read_to_string(agent.join("CLAUDE.md"))?;
        assert!(guide.contains("- `send_draft` (**writes**)") && !guide.contains("{TOOLS}"));
        let claude: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(
            agent.join(".claude/settings.json"),
        )?)?;
        let listed = |key: &str, name: &str| {
            claude["permissions"][key]
                .as_array()
                .is_some_and(|names| names.iter().any(|item| item == name))
        };
        assert!(
            listed("allow", "mcp__moonmail__search") && listed("ask", "mcp__moonmail__send_draft")
        );
        assert!(!listed("allow", "mcp__moonmail__app_action"));
        for config in [
            ".mcp.json",
            "opencode.json",
            ".gemini/settings.json",
            ".codex/config.toml",
        ] {
            assert!(std::fs::read_to_string(agent.join(config))?.contains("moonmail"));
        }
        Ok(())
    }
}
