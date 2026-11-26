//! Registered desktop OAuth clients (Google and Microsoft). Endpoints and scopes
//! are fixed here, never trusted from imported client JSON or a token record.
use super::*;

#[derive(Debug, Serialize, Deserialize)]
struct DesktopClient {
    client_id: String,
    #[serde(default = "empty_secret", serialize_with = "serialize_secret")]
    client_secret: SecretString,
}

struct Endpoints {
    key: &'static str,
    authorize: &'static str,
    token: &'static str,
    scope: &'static str,
}

fn endpoints<E: std::error::Error + 'static>(
    provider: Provider,
) -> Result<Endpoints, AuthError<E>> {
    match provider {
        Provider::Google => Ok(Endpoints {
            key: "google",
            authorize: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token",
            scope: "https://mail.google.com/",
        }),
        Provider::Microsoft => Ok(Endpoints {
            key: "microsoft",
            authorize: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize",
            token: "https://login.microsoftonline.com/common/oauth2/v2.0/token",
            scope: "https://outlook.office.com/IMAP.AccessAsUser.All https://outlook.office.com/SMTP.Send offline_access",
        }),
        _ => Err(AuthError::Protocol(
            "This provider does not use an imported OAuth client",
        )),
    }
}

fn parse_client<E: std::error::Error + 'static>(
    provider: Provider,
    bytes: &[u8],
) -> Result<DesktopClient, AuthError<E>> {
    if bytes.len() > 65536 {
        return Err(AuthError::Protocol("OAuth client file exceeds 64 KiB"));
    }
    let client: DesktopClient = if provider == Provider::Google {
        #[derive(Deserialize)]
        struct Download {
            installed: DesktopClient,
        }
        serde_json::from_slice::<Download>(bytes)?.installed
    } else {
        serde_json::from_slice(bytes)?
    };
    if client.client_id.is_empty()
        || client.client_id.len() > 512
        || client.client_id.chars().any(char::is_whitespace)
        || client.client_secret.expose_secret().len() > 4096
        || client
            .client_secret
            .expose_secret()
            .chars()
            .any(char::is_control)
        || (provider == Provider::Google
            && !client.client_id.ends_with(".apps.googleusercontent.com"))
        || (provider == Provider::Microsoft && uuid::Uuid::parse_str(&client.client_id).is_err())
    {
        return Err(AuthError::Protocol(
            "Import credentials for a registered desktop OAuth application",
        ));
    }
    Ok(client)
}

/// Packagers may supply the registered public desktop client before signing the app.
/// An explicit Keychain import takes precedence, including for development builds.
fn packaged_google_client<E: std::error::Error + 'static>(
    executable: &std::path::Path,
) -> Result<Option<DesktopClient>, AuthError<E>> {
    use std::{
        fs::File,
        io::{ErrorKind, Read},
    };
    let Some(macos) = executable
        .parent()
        .filter(|p| p.ends_with("Contents/MacOS"))
    else {
        return Ok(None);
    };
    let Some(contents) = macos.parent() else {
        return Ok(None);
    };
    if contents.parent().and_then(std::path::Path::extension) != Some(std::ffi::OsStr::new("app")) {
        return Ok(None);
    }
    let file = match File::open(contents.join("Resources/google-desktop-client.json")) {
        Ok(file) => file,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let mut data = String::new();
    file.take(65537).read_to_string(&mut data)?;
    let secret = SecretString::from(data);
    parse_client(Provider::Google, secret.expose_secret().as_bytes()).map(Some)
}

fn resolve_desktop_client<E: std::error::Error + 'static>(
    provider: Provider,
    imported: Option<SecretString>,
    executable: impl FnOnce() -> std::io::Result<std::path::PathBuf>,
) -> Result<DesktopClient, AuthError<E>> {
    if let Some(json) = imported {
        return Ok(serde_json::from_str(json.expose_secret())?);
    }
    if provider == Provider::Google
        && let Some(client) = packaged_google_client(&executable()?)?
    {
        return Ok(client);
    }
    Err(AuthError::Protocol(
        "Import this provider's desktop OAuth client in Internet Accounts first, or use an app password",
    ))
}

fn apply_tokens<E: std::error::Error + 'static>(
    record: &mut Record,
    tokens: &Tokens,
    initial: bool,
) -> Result<(), AuthError<E>> {
    if !tokens.token_type.eq_ignore_ascii_case("bearer")
        || tokens.access_token.expose_secret().is_empty()
        || tokens.expires_in == 0
    {
        return Err(AuthError::Protocol(
            "OAuth returned an incomplete token response; reconnect the account",
        ));
    }
    if record.provider == "google"
        && (initial || tokens.scope.is_some())
        && !tokens.scope.as_deref().is_some_and(|scope| {
            scope
                .split_whitespace()
                .any(|s| s == "https://mail.google.com/")
        })
    {
        return Err(AuthError::Protocol(
            "Google did not grant IMAP/SMTP mail access",
        ));
    }
    if initial && tokens.refresh_token.expose_secret().is_empty() {
        return Err(AuthError::Protocol(
            "Offline access was not granted; reconnect and consent again",
        ));
    }
    record.access_token.clone_from(&tokens.access_token);
    // Google normally omits refresh_token on refresh; Microsoft can rotate it.
    if !tokens.refresh_token.expose_secret().is_empty() {
        record.refresh_token.clone_from(&tokens.refresh_token);
    }
    record.expires_at = time::OffsetDateTime::now_utc()
        .unix_timestamp()
        .saturating_add(tokens.expires_in.min(i64::MAX as u64) as i64);
    record.refreshing = false;
    Ok(())
}

impl<C: CredentialStore> Auth<C> {
    pub fn import_oauth_client(
        &self,
        provider: Provider,
        bytes: &[u8],
    ) -> Result<(), AuthError<C::Error>> {
        let config = endpoints(provider)?;
        let client = parse_client(provider, bytes)?;
        let key = format!("oauth-client:{}", config.key);
        let _lock = self.store.lock(&key).map_err(AuthError::Credentials)?;
        let json = SecretString::from(serde_json::to_string(&client)?);
        self.store.save(&key, &json).map_err(AuthError::Credentials)
    }

    pub fn authorize_provider(
        &self,
        provider: Provider,
        email: &str,
    ) -> Result<(), AuthError<C::Error>> {
        if provider == Provider::Fastmail {
            return self.authorize(email);
        }
        let _lock = self.store.lock(email).map_err(AuthError::Credentials)?;
        let config = endpoints(provider)?;
        tracing::debug!(provider = config.key, "starting OAuth sign-in");
        let key = format!("oauth-client:{}", config.key);
        let desktop: DesktopClient = {
            let _lock = self.store.lock(&key).map_err(AuthError::Credentials)?;
            let imported = self.store.load(&key).map_err(AuthError::Credentials)?;
            resolve_desktop_client(provider, imported, std::env::current_exe)?
        };
        let client = self.client()?;
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        // Microsoft ignores dynamic ports for registered localhost desktop
        // redirects; Google recommends the literal loopback address.
        let host = if provider == Provider::Microsoft {
            "localhost"
        } else {
            "127.0.0.1"
        };
        let redirect = format!("http://{host}:{}/", listener.local_addr()?.port());
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let state = CsrfToken::new_random();
        let mut url = url::Url::parse(config.authorize)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", desktop.client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", config.scope),
            ("state", state.secret()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("login_hint", email),
            ("prompt", "consent"),
        ]);
        if provider == Provider::Google {
            url.query_pairs_mut().append_pair("access_type", "offline");
        }
        tracing::debug!(provider = config.key, %redirect, "opening the browser for OAuth");
        open::that(url.as_str())?;
        let code = receive_code(&listener, state.secret(), &redirect, provider)?;
        tracing::debug!(provider = config.key, "OAuth code received; exchanging it");
        let mut form = vec![
            ("client_id", desktop.client_id.as_str()),
            ("grant_type", "authorization_code"),
            ("redirect_uri", redirect.as_str()),
            ("code", code.expose_secret()),
            ("code_verifier", verifier.secret()),
        ];
        if !desktop.client_secret.expose_secret().is_empty() {
            form.push(("client_secret", desktop.client_secret.expose_secret()));
        }
        let tokens: Tokens = client
            .post(config.token)
            .form(&form)
            .send()?
            .error_for_status()?
            .json()?;
        let mut record = Record {
            provider: config.key.into(),
            client_id: desktop.client_id.clone(),
            client_secret: desktop.client_secret.clone(),
            password: empty_secret(),
            access_token: empty_secret(),
            refresh_token: empty_secret(),
            expires_at: 0,
            refreshing: false,
        };
        apply_tokens(&mut record, &tokens, true)?;
        tracing::debug!(provider = config.key, "OAuth tokens received; saving them");
        self.save(email, &record)
    }

    pub(super) fn refresh_external(
        &self,
        email: &str,
        record: &mut Record,
    ) -> Result<LoginSecret, AuthError<C::Error>> {
        let provider = if record.provider == "google" {
            Provider::Google
        } else {
            Provider::Microsoft
        };
        let config = endpoints(provider)?;
        let client = self.client()?;
        record.refreshing = true;
        self.save(email, record)?;
        let mut form = vec![
            ("client_id", record.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", record.refresh_token.expose_secret()),
        ];
        if !record.client_secret.expose_secret().is_empty() {
            form.push(("client_secret", record.client_secret.expose_secret()));
        }
        let tokens: Tokens = client
            .post(config.token)
            .form(&form)
            .send()?
            .error_for_status()?
            .json()?;
        apply_tokens(record, &tokens, false)?;
        self.save(email, record)?;
        Ok(LoginSecret {
            bearer: true,
            value: tokens.access_token.clone(),
        })
    }

    pub(super) fn revoke_google(&self, record: &Record) -> Result<(), AuthError<C::Error>> {
        if !record.refresh_token.expose_secret().is_empty() {
            self.client()?
                .post("https://oauth2.googleapis.com/revoke")
                .form(&[("token", record.refresh_token.expose_secret())])
                .send()?
                .error_for_status()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    type TestResult = Result<(), Box<dyn std::error::Error>>;

    #[test]
    fn packaged_desktop_client_is_scoped_to_an_app_bundle() -> Result<(), Box<dyn std::error::Error>>
    {
        let dir = tempfile::tempdir()?;
        let contents = dir.path().join("Moonmail.app/Contents");
        let exe = contents.join("MacOS/moonmail");
        assert!(packaged_google_client::<std::io::Error>(&exe)?.is_none());
        std::fs::create_dir_all(contents.join("Resources"))?;
        let path = contents.join("Resources/google-desktop-client.json");
        std::fs::write(
            &path,
            br#"{"installed":{"client_id":"test.apps.googleusercontent.com"}}"#,
        )?;
        assert!(packaged_google_client::<std::io::Error>(&exe)?.is_some());
        assert!(packaged_google_client::<std::io::Error>(&dir.path().join("moonmail"))?.is_none());
        std::fs::write(path, vec![b'a'; 65537])?;
        assert!(packaged_google_client::<std::io::Error>(&exe).is_err());
        Ok(())
    }

    #[test]
    fn imported_client_precedes_the_google_bundle_and_missing_clients_require_import() -> TestResult
    {
        let dir = tempfile::tempdir()?;
        let contents = dir.path().join("Moonmail.app/Contents");
        let resources = contents.join("Resources");
        std::fs::create_dir_all(&resources)?;
        let client_file = resources.join("google-desktop-client.json");
        let resolve = |provider, imported| {
            resolve_desktop_client::<std::io::Error>(provider, imported, || {
                Ok(contents.join("MacOS/moonmail"))
            })
        };
        assert!(matches!(
            resolve(Provider::Google, None),
            Err(AuthError::Protocol(_))
        ));
        std::fs::write(
            &client_file,
            br#"{"installed":{"client_id":"bundle.apps.googleusercontent.com"}}"#,
        )?;
        assert_eq!(
            resolve(Provider::Google, None)?.client_id,
            "bundle.apps.googleusercontent.com"
        );
        assert!(matches!(
            resolve(Provider::Microsoft, None),
            Err(AuthError::Protocol(_))
        ));
        assert!(matches!(
            resolve(Provider::Google, Some("invalid".into())),
            Err(AuthError::Json(_))
        ));
        std::fs::write(client_file, b"invalid")?;
        let imported = SecretString::from(r#"{"client_id":"imported.apps.googleusercontent.com"}"#);
        assert_eq!(
            resolve(Provider::Google, Some(imported))?.client_id,
            "imported.apps.googleusercontent.com"
        );
        Ok(())
    }

    #[test]
    fn refresh_keeps_google_refresh_token_and_accepts_rotation() -> TestResult {
        let mut record = Record {
            provider: "google".into(),
            refresh_token: "original".into(),
            refreshing: true,
            client_id: String::new(),
            client_secret: empty_secret(),
            password: empty_secret(),
            access_token: empty_secret(),
            expires_at: 0,
        };
        let mut tokens: Tokens = serde_json::from_str(
            r#"{"access_token":"new","expires_in":3600,"token_type":"Bearer"}"#,
        )?;
        apply_tokens::<std::io::Error>(&mut record, &tokens, false)?;
        assert_eq!(record.refresh_token.expose_secret(), "original");
        assert!(!record.refreshing);
        tokens.refresh_token = "rotated".into();
        apply_tokens::<std::io::Error>(&mut record, &tokens, false)?;
        assert_eq!(record.refresh_token.expose_secret(), "rotated");
        assert!(apply_tokens::<std::io::Error>(&mut record, &tokens, true).is_err());
        tokens.scope = Some("https://mail.google.com/".into());
        apply_tokens::<std::io::Error>(&mut record, &tokens, true)?;
        Ok(())
    }

    #[test]
    fn desktop_import_and_provider_bound_callbacks_reject_wrong_clients() -> TestResult {
        let client = parse_client::<std::io::Error>(Provider::Google, br#"{"installed":{"client_id":"synthetic.apps.googleusercontent.com","client_secret":"synthetic"}}"#)?;
        assert_eq!(client.client_secret.expose_secret(), "synthetic");
        assert!(
            parse_client::<std::io::Error>(Provider::Google, br#"{"web":{"client_id":"wrong"}}"#)
                .is_err()
        );
        assert!(
            parse_client::<std::io::Error>(Provider::Microsoft, br#"{"client_id":"not-a-uuid"}"#)
                .is_err()
        );
        let redirect = "http://127.0.0.1:54321/";
        assert!(matches!(
            parse_callback_for(
                "/?state=state&code=code",
                "state",
                redirect,
                Provider::Google
            ),
            Some(Callback::Code(_))
        ));
        for target in [
            "/?state=wrong&code=code",
            "/?state=state&code=code&error=denied",
            "/?state=state&code=code&iss=https://evil.example",
            "/wrong?state=state&code=code",
        ] {
            assert!(parse_callback_for(target, "state", redirect, Provider::Google).is_none());
        }
        Ok(())
    }
}
