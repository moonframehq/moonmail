//! Email-first provider discovery. Unknown/custom domains use the system DNS
//! resolver's MX records; recognition never implies that sign-in is supported.
use {
    anyhow::{Context, Result},
    serde::Serialize,
};

#[derive(Clone, Serialize)]
pub struct Discovery {
    pub email: String,
    pub provider: String,
    pub source: String,
}

pub fn provider(name: &str) -> Result<moonmail_network::Provider> {
    Ok(match name {
        "fastmail" | "" => moonmail_network::Provider::Fastmail,
        "google" => moonmail_network::Provider::Google,
        "microsoft" => moonmail_network::Provider::Microsoft,
        "icloud" => moonmail_network::Provider::Icloud,
        "yahoo" => moonmail_network::Provider::Yahoo,
        "aol" => moonmail_network::Provider::Aol,
        "other" => moonmail_network::Provider::Other,
        _ => anyhow::bail!("Unknown mail provider"),
    })
}

/// Use a native secure-entry dialog, rather than routing a password through the
/// Luau workspace, view snapshots, input events, or persisted app state.
#[cfg(target_os = "macos")]
pub fn prompt_password(email: &str) -> Result<moonmail_network::SecretString> {
    let output = std::process::Command::new("/usr/bin/osascript")
        .args(["-e", "on run argv\nset answer to display dialog (\"Enter the app password for \" & item 1 of argv) with title \"Connect to Moonmail\" default answer \"\" with hidden answer buttons {\"Cancel\", \"Connect\"} default button \"Connect\" cancel button \"Cancel\"\nreturn text returned of answer\nend run", email])
        .output()?;
    anyhow::ensure!(output.status.success(), "Account connection cancelled");
    use moonmail_network::ExposeSecret;
    let password = moonmail_network::SecretString::from(String::from_utf8(output.stdout)?);
    let entered = password.expose_secret().trim_end_matches('\n');
    anyhow::ensure!(!entered.is_empty(), "Enter an app password");
    Ok(entered.into())
}

#[cfg(not(target_os = "macos"))]
pub fn prompt_password(_email: &str) -> Result<moonmail_network::SecretString> {
    anyhow::bail!("Native password entry currently requires macOS")
}

/// The macOS session this process runs in, for logs: only `Aqua`, the login
/// session, can reach the Keychain and show its prompts.
pub fn session() -> String {
    std::process::Command::new("/bin/launchctl")
        .arg("managername")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned())
}

pub fn domain_provider(domain: &str) -> Option<&'static str> {
    match domain.to_ascii_lowercase().as_str() {
        "fastmail.com" | "fastmail.fm" | "messagingengine.com" | "pobox.com" => Some("fastmail"),
        "gmail.com" | "googlemail.com" => Some("google"),
        "icloud.com" | "me.com" | "mac.com" => Some("icloud"),
        "outlook.com" | "hotmail.com" | "live.com" | "msn.com" => Some("microsoft"),
        "yahoo.com" | "ymail.com" | "rocketmail.com" => Some("yahoo"),
        "aol.com" => Some("aol"),
        _ => None,
    }
}

fn mx_provider(records: &str) -> Option<&'static str> {
    let mut found = None;
    for line in records.lines() {
        let mut fields = line.split_whitespace();
        if fields.next().and_then(|s| s.parse::<u16>().ok()).is_none() {
            continue;
        }
        let Some(host) = fields.next() else {
            continue;
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let matches = |suffix: &str| host == suffix || host.ends_with(&format!(".{suffix}"));
        let provider = if matches("messagingengine.com") {
            Some("fastmail")
        } else if matches("google.com") || matches("googlemail.com") {
            Some("google")
        } else if matches("protection.outlook.com") {
            Some("microsoft")
        } else if matches("icloud.com") {
            Some("icloud")
        } else if matches("yahoodns.net") {
            Some("yahoo")
        } else {
            None
        };
        // Mixed/unknown MX records are not enough to choose an account provider.
        let provider = provider?;
        if found.is_some_and(|old| old != provider) {
            return None;
        }
        found = Some(provider);
    }
    found
}

pub fn discover(email: &str, lookup_mx: bool) -> Result<Discovery> {
    moonmail_network::validate_address(email)?;
    let (_, domain) = email.rsplit_once('@').context("Enter an email address")?;
    anyhow::ensure!(
        domain.len() <= 253
            && domain.split('.').all(|label| !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')),
        "Enter a valid email domain"
    );
    let (provider, source) = if let Some(provider) = domain_provider(domain) {
        (Some(provider), "email domain")
    } else if lookup_mx {
        // Fixed executable/options, validated DNS name, bounded resolver timeout.
        let output = std::process::Command::new("/usr/bin/dig")
            .args(["+short", "+time=2", "+tries=1", "MX", domain])
            .output();
        let provider = output
            .ok()
            .filter(|out| out.status.success())
            .and_then(|out| mx_provider(&String::from_utf8_lossy(&out.stdout)));
        (provider, "mail server records")
    } else {
        (None, "offline demo")
    };
    tracing::debug!(provider, source, "account discovery finished");
    Ok(Discovery {
        email: email.into(),
        provider: provider.unwrap_or("unknown").into(),
        source: source.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_matches_domains_and_mx_boundaries() -> Result<()> {
        assert_eq!(discover("person@FASTMAIL.COM", false)?.provider, "fastmail");
        assert_eq!(discover("person@custom.test", false)?.provider, "unknown");
        assert_eq!(domain_provider("notgmail.com"), None);
        assert_eq!(
            mx_provider("10 in1-smtp.messagingengine.com.\n20 in2-smtp.messagingengine.com."),
            Some("fastmail")
        );
        assert_eq!(mx_provider("10 evil-messagingengine.com."), None);
        assert_eq!(mx_provider("10 aspmx.l.google.com."), Some("google"));
        assert_eq!(
            mx_provider("10 in1-smtp.messagingengine.com.\n20 mail.unknown.test."),
            None
        );
        assert!(discover("person@-option.test", false).is_err());
        assert!(discover("not an email", false).is_err());
        Ok(())
    }
}
