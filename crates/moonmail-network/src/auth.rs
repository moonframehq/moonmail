use {
    crate::{CredentialStore, Provider},
    oauth2::{CsrfToken, PkceCodeChallenge},
    reqwest::blocking::Client,
    secrecy::{ExposeSecret, SecretString},
    serde::{Deserialize, Serialize},
    std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        time::{Duration, Instant},
    },
};

const ISSUER: &str = "https://api.fastmail.com";

#[path = "oauth_external.rs"]
mod external;

#[derive(Debug, thiserror::Error)]
pub enum AuthError<E: std::error::Error + 'static> {
    #[error("credential store failed")]
    Credentials(#[source] E),
    #[error("authentication network request failed")]
    Http(#[from] reqwest::Error),
    #[error("authentication file or browser operation failed")]
    Io(#[from] std::io::Error),
    #[error("invalid credential record")]
    Json(#[from] serde_json::Error),
    #[error("invalid OAuth URL")]
    Url(#[from] url::ParseError),
    #[error("{0}")]
    Protocol(&'static str),
}

/// Passwords and tokens stay in `SecretString`: `Debug` redacts them, and
/// memory is wiped on drop. Reading one takes an explicit `expose_secret`.
#[derive(Clone, Debug)]
pub struct LoginSecret {
    pub bearer: bool,
    pub value: SecretString,
}

fn empty_secret() -> SecretString {
    SecretString::from("")
}

/// The one way a secret reaches serialized form: the Keychain record.
fn serialize_secret<S: serde::Serializer>(
    secret: &SecretString,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(secret.expose_secret())
}

#[derive(Debug, Serialize, Deserialize)]
struct Record {
    #[serde(default)]
    provider: String,
    #[serde(default = "empty_secret", serialize_with = "serialize_secret")]
    client_secret: SecretString,
    #[serde(serialize_with = "serialize_secret")]
    password: SecretString,
    client_id: String,
    #[serde(serialize_with = "serialize_secret")]
    access_token: SecretString,
    #[serde(serialize_with = "serialize_secret")]
    refresh_token: SecretString,
    expires_at: i64,
    refreshing: bool,
}

#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    protected_resources: Option<Vec<String>>,
    registration_endpoint: String,
    authorization_endpoint: String,
    token_endpoint: String,
    revocation_endpoint: String,
}

#[derive(Deserialize)]
struct Registration {
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct Tokens {
    access_token: SecretString,
    #[serde(default = "empty_secret")]
    refresh_token: SecretString,
    expires_in: u64,
    token_type: String,
    #[serde(default)]
    scope: Option<String>,
}

pub struct Auth<C> {
    store: C,
}

impl<C: CredentialStore> Auth<C> {
    pub fn new(store: C) -> Self {
        Self { store }
    }

    fn client(&self) -> Result<Client, AuthError<C::Error>> {
        Ok(Client::builder()
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()?)
    }

    fn discovery(&self, client: &Client) -> Result<Discovery, AuthError<C::Error>> {
        let metadata: Discovery = client
            .get(format!("{ISSUER}/.well-known/oauth-authorization-server"))
            .send()?
            .error_for_status()?
            .json()?;
        if metadata.issuer != ISSUER
            || [
                &metadata.registration_endpoint,
                &metadata.authorization_endpoint,
                &metadata.token_endpoint,
                &metadata.revocation_endpoint,
            ]
            .iter()
            .any(|value| {
                url::Url::parse(value)
                    .map(|url| url.origin().ascii_serialization() != ISSUER)
                    .unwrap_or(true)
            })
        {
            return Err(AuthError::Protocol(
                "Fastmail returned an unexpected OAuth issuer or endpoint",
            ));
        }
        Ok(metadata)
    }

    fn save(&self, email: &str, record: &Record) -> Result<(), AuthError<C::Error>> {
        let json = SecretString::from(serde_json::to_string(record)?);
        self.store
            .save(email, &json)
            .map_err(AuthError::Credentials)
    }

    pub fn password(&self, email: &str, password: SecretString) -> Result<(), AuthError<C::Error>> {
        self.save_static(email, password, "")
    }

    pub fn api_token(&self, email: &str, token: SecretString) -> Result<(), AuthError<C::Error>> {
        self.save_static(email, token, "fastmail-jmap")
    }

    fn save_static(
        &self,
        email: &str,
        password: SecretString,
        provider: &str,
    ) -> Result<(), AuthError<C::Error>> {
        let _lock = self.store.lock(email).map_err(AuthError::Credentials)?;
        self.save(
            email,
            &Record {
                password,
                provider: provider.into(),
                client_secret: empty_secret(),
                client_id: String::new(),
                access_token: empty_secret(),
                refresh_token: empty_secret(),
                expires_at: 0,
                refreshing: false,
            },
        )
    }

    pub fn authorize(&self, email: &str) -> Result<(), AuthError<C::Error>> {
        self.authorize_fastmail(email, crate::Transport::Imap)
    }

    pub fn authorize_fastmail(
        &self,
        email: &str,
        transport: crate::Transport,
    ) -> Result<(), AuthError<C::Error>> {
        let scope = match transport {
            crate::Transport::Imap => "urn:ietf:params:oauth:scope:mail offline_access",
            crate::Transport::Jmap => {
                "urn:ietf:params:jmap:core urn:ietf:params:jmap:mail urn:ietf:params:jmap:submission offline_access"
            },
        };
        tracing::debug!(?transport, "starting Fastmail OAuth discovery");
        let _lock = self.store.lock(email).map_err(AuthError::Credentials)?;
        let client = self.client()?;
        let metadata = self.discovery(&client)?;
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let redirect = format!(
            "http://127.0.0.1:{}/oauth/fastmail",
            listener.local_addr()?.port()
        );
        tracing::debug!("registering Fastmail OAuth client");
        let registration: Registration = client
            .post(&metadata.registration_endpoint)
            .json(&serde_json::json!({
                "client_name":"Moonmail", "application_type":"native", "redirect_uris":[redirect],
                "grant_types":["authorization_code","refresh_token"], "response_types":["code"],
                "token_endpoint_auth_method":"none", "scope":scope
            }))
            .send()?
            .error_for_status()?
            .json()?;
        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let state = CsrfToken::new_random();
        let mut url = url::Url::parse(&metadata.authorization_endpoint)?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", registration.client_id.as_str()),
            ("redirect_uri", redirect.as_str()),
            ("response_type", "code"),
            ("scope", scope),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", state.secret()),
            ("prompt", "consent"),
            ("login_hint", email),
        ]);
        if transport == crate::Transport::Imap {
            url.query_pairs_mut().extend_pairs([
                ("resource", "imap.fastmail.com"),
                ("resource", "smtp.fastmail.com"),
            ]);
        } else {
            let endpoint = url::Url::parse(crate::jmap::SESSION)?;
            let resource = if let Some(resources) = &metadata.protected_resources {
                resources
                    .iter()
                    .filter_map(|resource| {
                        let parsed = url::Url::parse(resource).ok()?;
                        let prefix = parsed.path().trim_end_matches('/');
                        let suffix = endpoint.path().strip_prefix(prefix)?;
                        (parsed.origin() == endpoint.origin()
                            && parsed.username().is_empty()
                            && parsed.password().is_none()
                            && parsed.fragment().is_none()
                            && parsed.query().is_none()
                            && (suffix.is_empty() || suffix.starts_with('/')))
                        .then_some((prefix.len(), resource.as_str()))
                    })
                    .max_by_key(|(length, _)| *length)
                    .ok_or(AuthError::Protocol(
                        "Fastmail does not advertise its JMAP resource",
                    ))?
                    .1
            } else {
                crate::jmap::SESSION
            };
            url.query_pairs_mut().append_pair("resource", resource);
        }
        tracing::debug!("opening browser for Fastmail sign-in");
        open::that(url.as_str())?;
        let code = receive_code(&listener, state.secret(), &redirect, Provider::Fastmail)?;
        tracing::debug!("Fastmail callback accepted, exchanging authorization code");
        let tokens: Tokens = client
            .post(&metadata.token_endpoint)
            .form(&[
                ("client_id", registration.client_id.as_str()),
                ("grant_type", "authorization_code"),
                ("redirect_uri", redirect.as_str()),
                ("code", code.expose_secret()),
                ("code_verifier", verifier.secret()),
            ])
            .send()?
            .error_for_status()?
            .json()?;
        tracing::debug!("Fastmail token exchange complete, saving credentials");
        self.persist_tokens(email, registration.client_id, &tokens)
    }

    fn persist_tokens(
        &self,
        email: &str,
        client_id: String,
        tokens: &Tokens,
    ) -> Result<(), AuthError<C::Error>> {
        if !tokens.token_type.eq_ignore_ascii_case("bearer")
            || tokens.access_token.expose_secret().is_empty()
            || tokens.refresh_token.expose_secret().is_empty()
        {
            return Err(AuthError::Protocol(
                "Fastmail returned an incomplete token response; reconnect the account",
            ));
        }
        let expires_at = time::OffsetDateTime::now_utc()
            .unix_timestamp()
            .saturating_add(tokens.expires_in.min(i64::MAX as u64) as i64);
        self.save(
            email,
            &Record {
                password: empty_secret(),
                client_id,
                access_token: tokens.access_token.clone(),
                refresh_token: tokens.refresh_token.clone(),
                expires_at,
                refreshing: false,
                provider: String::new(),
                client_secret: empty_secret(),
            },
        )
    }

    pub fn credentials(&self, email: &str) -> Result<LoginSecret, AuthError<C::Error>> {
        self.credentials_bound(email, None)
    }

    pub fn credentials_for(
        &self,
        account: &crate::Account,
    ) -> Result<LoginSecret, AuthError<C::Error>> {
        self.credentials_bound(&account.email, Some(account))
    }

    fn credentials_bound(
        &self,
        email: &str,
        account: Option<&crate::Account>,
    ) -> Result<LoginSecret, AuthError<C::Error>> {
        let _lock = self.store.lock(email).map_err(AuthError::Credentials)?;
        let json = self
            .store
            .load(email)
            .map_err(AuthError::Credentials)?
            .ok_or(AuthError::Protocol("Connect your account first"))?;
        let mut record: Record = serde_json::from_str(json.expose_secret())?;
        if !matches!(
            record.provider.as_str(),
            "" | "fastmail" | "fastmail-jmap" | "google" | "microsoft"
        ) {
            return Err(AuthError::Protocol(
                "Unrecognized credential provider; reconnect the account",
            ));
        }
        if !record.password.expose_secret().is_empty() {
            let jmap = record.provider == "fastmail-jmap";
            if jmap
                != account.is_some_and(|a| {
                    a.transport == crate::Transport::Jmap && a.provider == Provider::Fastmail
                })
            {
                return Err(AuthError::Protocol(
                    "Reconnect with credentials for this transport",
                ));
            }
            return Ok(LoginSecret {
                bearer: jmap,
                value: record.password.clone(),
            });
        }
        if let Some(account) = account {
            let provider = match record.provider.as_str() {
                "google" => Provider::Google,
                "microsoft" => Provider::Microsoft,
                _ => Provider::Fastmail,
            };
            let preset = crate::Account::preset(provider, email)
                .map_err(|_| AuthError::Protocol("Invalid OAuth account address"))?;
            if account.provider != provider
                || account.imap_host != preset.imap_host
                || account.smtp_host != preset.smtp_host
                || account.imap_port != preset.imap_port
                || account.smtp_port != preset.smtp_port
                || account.imap_tls != preset.imap_tls
                || account.smtp_tls != preset.smtp_tls
                || account.imap_login() != email
                || account.smtp_login() != email
            {
                return Err(AuthError::Protocol(
                    "OAuth credentials require their original provider's server settings; reconnect this account",
                ));
            }
        }
        if record.refreshing {
            return Err(AuthError::Protocol(
                "A previous token refresh was interrupted. Reconnect to avoid reusing a rotated refresh token",
            ));
        }
        if record.expires_at > time::OffsetDateTime::now_utc().unix_timestamp() + 90 {
            return Ok(LoginSecret {
                bearer: true,
                value: record.access_token.clone(),
            });
        }
        if matches!(record.provider.as_str(), "google" | "microsoft") {
            return self.refresh_external(email, &mut record);
        }
        let client = self.client()?;
        let metadata = self.discovery(&client)?;
        // Persist intent before exchange. After an ambiguous crash or failed save,
        // an old rotating token is never silently retried.
        record.refreshing = true;
        self.save(email, &record)?;
        let tokens: Tokens = client
            .post(&metadata.token_endpoint)
            .form(&[
                ("client_id", record.client_id.as_str()),
                ("grant_type", "refresh_token"),
                ("refresh_token", record.refresh_token.expose_secret()),
            ])
            .send()?
            .error_for_status()?
            .json()?;
        self.persist_tokens(email, record.client_id.clone(), &tokens)?;
        Ok(LoginSecret {
            bearer: true,
            value: tokens.access_token.clone(),
        })
    }

    pub fn disconnect(&self, email: &str) -> Result<(), AuthError<C::Error>> {
        let _lock = self.store.lock(email).map_err(AuthError::Credentials)?;
        if let Some(json) = self.store.load(email).map_err(AuthError::Credentials)? {
            let record: Record = serde_json::from_str(json.expose_secret())?;
            if record.provider == "google" {
                self.revoke_google(&record)?;
            } else if record.provider == "microsoft" {
                // Microsoft's delegated desktop flow has no per-token revocation
                // endpoint. Disconnect removes this app's locally held grant.
            } else if !record.refresh_token.expose_secret().is_empty() {
                let client = self.client()?;
                let metadata = self.discovery(&client)?;
                client
                    .post(metadata.revocation_endpoint)
                    .form(&[
                        ("token", record.refresh_token.expose_secret()),
                        ("client_id", record.client_id.as_str()),
                        ("token_type_hint", "refresh_token"),
                    ])
                    .send()?
                    .error_for_status()?;
            }
        }
        self.store.delete(email).map_err(AuthError::Credentials)
    }
}

fn receive_code<E: std::error::Error + 'static>(
    listener: &TcpListener,
    state: &str,
    redirect: &str,
    provider: Provider,
) -> Result<SecretString, AuthError<E>> {
    let deadline = Instant::now() + Duration::from_secs(300);
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                let mut line = String::new();
                if BufReader::new((&stream).take(8192))
                    .read_line(&mut line)
                    .is_err()
                {
                    continue;
                }
                let Some(target) = line
                    .strip_prefix("GET ")
                    .and_then(|rest| rest.split_whitespace().next())
                else {
                    continue;
                };
                let Some(callback) = parse_callback_for(target, state, redirect, provider) else {
                    let _=stream.write_all(b"HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\nInvalid OAuth callback.");
                    continue;
                };
                if matches!(callback, Callback::Denied) {
                    let _=stream.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nAuthorization declined. Return to Moonmail.");
                    return Err(AuthError::Protocol("Authorization was declined"));
                }
                if let Callback::Code(code) = callback {
                    let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nAuthorization received. You can return to Moonmail.");
                    return Ok(code);
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            },
            Err(error) => return Err(error.into()),
        }
    }
    Err(AuthError::Protocol(
        "Authorization timed out after five minutes",
    ))
}

enum Callback {
    Denied,
    Code(SecretString),
}

fn parse_callback_for(
    target: &str,
    state: &str,
    redirect: &str,
    provider: Provider,
) -> Option<Callback> {
    if !target.starts_with('/') || target.starts_with("//") {
        return None;
    }
    let expected = url::Url::parse(redirect).ok()?;
    let callback = expected.join(target).ok()?;
    if callback.origin() != expected.origin() || callback.path() != expected.path() {
        return None;
    }
    let mut parameters = std::collections::HashMap::new();
    for (key, value) in callback.query_pairs() {
        if parameters.insert(key, value).is_some() {
            return None;
        }
    }
    if parameters.get("state")?.as_ref() != state {
        return None;
    }
    let issuer = parameters.get("iss").map(|value| value.as_ref());
    match provider {
        Provider::Fastmail if issuer != Some(ISSUER) => return None,
        Provider::Google if issuer.is_some_and(|value| value != "https://accounts.google.com") => {
            return None;
        },
        Provider::Microsoft
            if issuer.is_some_and(|value| {
                url::Url::parse(value).map_or(true, |url| {
                    url.scheme() != "https"
                        || url.host_str() != Some("login.microsoftonline.com")
                        || !url.path().ends_with("/v2.0")
                })
            }) =>
        {
            return None;
        },
        Provider::Fastmail | Provider::Google | Provider::Microsoft => {},
        _ => return None,
    }
    if parameters.contains_key("code") && parameters.contains_key("error") {
        return None;
    }
    if parameters.contains_key("error") {
        return Some(Callback::Denied);
    }
    let code = parameters.get("code")?;
    if code.is_empty() {
        return None;
    }
    Some(Callback::Code(code.to_string().into()))
}

pub fn oauth_bearer(host: &str, port: u16, token: &SecretString) -> SecretString {
    bearer_payload("", host, port, token)
}

pub(crate) fn account_bearer(
    login: &str,
    host: &str,
    port: u16,
    token: &SecretString,
) -> SecretString {
    // RFC 5801 saslname escaping for the GS2 authorization identity.
    let identity = login.replace('=', "=3D").replace(',', "=2C");
    bearer_payload(&format!("a={identity}"), host, port, token)
}

fn bearer_payload(identity: &str, host: &str, port: u16, token: &SecretString) -> SecretString {
    let token = token.expose_secret();
    format!("n,{identity},\x01host={host}\x01port={port}\x01auth=Bearer {token}\x01\x01").into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_includes_escaped_authorization_identity() {
        let payload = account_bearer(
            "one=two,three@example.test",
            "imap.fastmail.com",
            993,
            &"synthetic".into(),
        );
        assert_eq!(
            payload.expose_secret(),
            "n,a=one=3Dtwo=2Cthree@example.test,\x01host=imap.fastmail.com\x01port=993\x01auth=Bearer synthetic\x01\x01"
        );
    }

    fn parse_callback(target: &str, state: &str, redirect: &str) -> Option<Callback> {
        parse_callback_for(target, state, redirect, Provider::Fastmail)
    }

    #[test]
    fn callbacks_require_state_issuer_path_and_unambiguous_parameters() {
        let redirect = "http://127.0.0.1:43210/oauth/fastmail";
        let good = "/oauth/fastmail?state=secret&iss=https%3A%2F%2Fapi.fastmail.com&code=code";
        assert!(matches!(
            parse_callback(good, "secret", redirect),
            Some(Callback::Code(_))
        ));
        for bad in [
            good.replace("state=secret", "state=wrong"),
            good.replace("api.fastmail.com", "evil.example"),
            good.replace("/oauth/fastmail", "/wrong"),
            format!("{good}&state=secret"),
            good.replace("code=code", "code="),
            format!("//evil.example{good}"),
        ] {
            assert!(parse_callback(&bad, "secret", redirect).is_none(), "{bad}");
        }
        assert!(matches!(
            parse_callback(
                &good.replace("code=code", "error=access_denied"),
                "secret",
                redirect
            ),
            Some(Callback::Denied)
        ));
    }

    struct Stored(String);
    impl CredentialStore for Stored {
        type Error = std::io::Error;
        type Guard = ();
        fn lock(&self, _: &str) -> Result<Self::Guard, Self::Error> {
            Ok(())
        }
        fn load(&self, _: &str) -> Result<Option<SecretString>, Self::Error> {
            Ok(Some(self.0.clone().into()))
        }
        fn save(&self, _: &str, _: &SecretString) -> Result<(), Self::Error> {
            panic!("interrupted refresh must not write or retry")
        }
        fn delete(&self, _: &str) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    #[test]
    fn oauth_credentials_cannot_be_retargeted_to_another_provider_or_host()
    -> Result<(), Box<dyn std::error::Error>> {
        let auth = Auth::new(Stored(serde_json::json!({
            "provider":"google", "password":"", "client_id":"synthetic.apps.googleusercontent.com",
            "access_token":"synthetic", "refresh_token":"refresh", "expires_at":i64::MAX, "refreshing":false
        }).to_string()));
        let mut account = crate::Account::preset(Provider::Google, "me@example.org")?;
        assert!(auth.credentials_for(&account)?.bearer);
        account.smtp_host = "another.example.org".into();
        assert!(auth.credentials_for(&account).is_err());
        assert!(
            auth.credentials_for(&crate::Account::fastmail("me@example.org")?)
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn debug_output_redacts_every_stored_secret_but_the_keychain_keeps_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let record = Record {
            password: "synthetic-password".into(),
            client_id: "id".into(),
            access_token: "synthetic-access".into(),
            refresh_token: "synthetic-refresh".into(),
            expires_at: 0,
            refreshing: false,
            provider: String::new(),
            client_secret: "synthetic-client".into(),
        };
        let tokens: Tokens = serde_json::from_str(
            r#"{"access_token":"synthetic-access","refresh_token":"synthetic-refresh","expires_in":1,"token_type":"Bearer"}"#,
        )?;
        let login = LoginSecret {
            bearer: true,
            value: "synthetic-access".into(),
        };
        let logged = format!("{record:?} {tokens:?} {login:?}");
        assert!(!logged.contains("synthetic-"), "{logged}");

        let stored = serde_json::to_string(&record)?;
        let restored: Record = serde_json::from_str(&stored)?;
        assert_eq!(restored.password.expose_secret(), "synthetic-password");
        assert_eq!(restored.client_secret.expose_secret(), "synthetic-client");
        assert_eq!(restored.access_token.expose_secret(), "synthetic-access");
        assert_eq!(restored.refresh_token.expose_secret(), "synthetic-refresh");
        Ok(())
    }

    #[test]
    fn interrupted_rotating_refresh_requires_reconnect_without_network()
    -> Result<(), Box<dyn std::error::Error>> {
        let record = Record {
            password: empty_secret(),
            client_id: "id".into(),
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at: 0,
            refreshing: true,
            provider: String::new(),
            client_secret: empty_secret(),
        };
        let auth = Auth::new(Stored(serde_json::to_string(&record)?));
        assert!(matches!(
            auth.credentials("test@example.com"),
            Err(AuthError::Protocol(_))
        ));
        Ok(())
    }
}
