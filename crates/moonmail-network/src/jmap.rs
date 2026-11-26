//! Blocking, bearer-authenticated JMAP connection for a network worker.
use {
    crate::{ExposeSecret, SecretString},
    reqwest::blocking::{Client as Http, RequestBuilder, Response},
    serde::{Deserialize, de::DeserializeOwned},
    serde_json::{Value, json},
    std::{collections::HashMap, io::Read, time::Duration},
    url::Url,
};

pub(crate) const CORE: &str = "urn:ietf:params:jmap:core";
pub(crate) const MAIL: &str = "urn:ietf:params:jmap:mail";
const SUBMISSION: &str = "urn:ietf:params:jmap:submission";
pub(crate) const SESSION: &str = "https://api.fastmail.com/jmap/session";

#[derive(Debug, thiserror::Error)]
pub enum JmapError {
    #[error("JMAP transport failed")]
    Http(#[from] reqwest::Error),
    #[error("JMAP response read failed")]
    Io(#[from] std::io::Error),
    #[error("Invalid JMAP response")]
    Json(#[from] serde_json::Error),
    #[error("JMAP credentials expired or lack permission")]
    Unauthorized,
    #[error("JMAP object no longer exists")]
    NotFound,
    #[error("JMAP state expired")]
    StateExpired,
    #[error("JMAP server could not confirm the outcome")]
    ServerFailure,
    #[error("JMAP server rejected the operation")]
    Rejected,
    #[error("Invalid or incomplete JMAP response")]
    Protocol,
}
pub(crate) type Result<T> = std::result::Result<T, JmapError>;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Session {
    api_url: String,
    download_url: String,
    upload_url: String,
    event_source_url: String,
    primary_accounts: HashMap<String, String>,
    capabilities: HashMap<String, Value>,
}

pub struct Client {
    http: Http,
    token: SecretString,
    origin: Url,
    session: Session,
    pub(crate) account: String,
    pub(crate) batch: usize,
}

pub(crate) fn field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key].as_str().ok_or(JmapError::Protocol)
}

pub(crate) fn bounded(mut response: Response, limit: u64) -> Result<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > limit) {
        return Err(JmapError::Protocol);
    }
    let mut bytes = Vec::new();
    (&mut response).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(JmapError::Protocol);
    }
    Ok(bytes)
}
fn checked(response: Response) -> Result<Response> {
    if matches!(response.status().as_u16(), 401 | 403) {
        return Err(JmapError::Unauthorized);
    }
    if !response.status().is_success() {
        return Err(response
            .error_for_status()
            .err()
            .map_or(JmapError::Protocol, |e| JmapError::Http(e.without_url())));
    }
    Ok(response)
}
pub(crate) fn json_body<T: DeserializeOwned>(response: Response) -> Result<T> {
    Ok(serde_json::from_slice(&bounded(
        response,
        16 * 1024 * 1024,
    )?)?)
}
impl Client {
    pub fn account_id(&self) -> &str {
        &self.account
    }

    pub fn fastmail(token: SecretString) -> Result<Self> {
        Self::discover(SESSION, token)
    }
    /// The supplied session URL is the trust anchor. HTTP is limited to loopback fixtures.
    pub fn discover(session_url: &str, token: SecretString) -> Result<Self> {
        let origin = Url::parse(session_url).map_err(|_| JmapError::Protocol)?;
        if !(origin.scheme() == "https"
            || origin.scheme() == "http" && origin.host_str() == Some("127.0.0.1"))
            || !origin.username().is_empty()
            || origin.password().is_some()
        {
            return Err(JmapError::Protocol);
        }
        let http = Http::builder()
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let get = |url| {
            http.get(url)
                .bearer_auth(token.expose_secret())
                .send()
                .map_err(|e| JmapError::Http(e.without_url()))
        };
        let mut response = get(origin.clone())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND
            && origin.path() != "/.well-known/jmap"
        {
            response = get(origin
                .join("/.well-known/jmap")
                .map_err(|_| JmapError::Protocol)?)?;
        }
        for _ in 0..3 {
            if !response.status().is_redirection() {
                break;
            }
            let location = response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok())
                .ok_or(JmapError::Protocol)?;
            let url = response
                .url()
                .join(location)
                .map_err(|_| JmapError::Protocol)?;
            if url.origin() != origin.origin()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(JmapError::Protocol);
            }
            response = get(url)?;
        }
        let session: Session = json_body(checked(response)?)?;
        let account = session
            .primary_accounts
            .get(MAIL)
            .filter(|id| !id.is_empty())
            .ok_or(JmapError::Protocol)?
            .clone();
        let batch = session
            .capabilities
            .get(CORE)
            .and_then(|c| c["maxObjectsInGet"].as_u64())
            .filter(|n| *n > 0)
            .ok_or(JmapError::Protocol)?
            .min(128) as usize;
        Ok(Self {
            http,
            token,
            session,
            origin,
            account,
            batch,
        })
    }
    fn endpoint(&self, template: &str, blob: &str) -> Result<Url> {
        let encode = |v: &str| {
            url::form_urlencoded::byte_serialize(v.as_bytes())
                .collect::<String>()
                .replace('+', "%20")
        };
        let expanded = template
            .replace("{accountId}", &encode(&self.account))
            .replace("{blobId}", &encode(blob))
            .replace("{name}", "message.eml")
            .replace("{type}", "message%2Frfc822")
            .replace("{types}", "Mailbox,Email")
            .replace("{closeafter}", "state")
            .replace("{ping}", "30");
        let url = self
            .origin
            .join(&expanded)
            .map_err(|_| JmapError::Protocol)?;
        let fastmail = self.origin.origin().ascii_serialization() == "https://api.fastmail.com"
            && url.scheme() == "https"
            && url.port().is_none()
            && url.host_str().is_some_and(|host| {
                host.ends_with(".api.fastmail.com")
                    || host == "jmap.fastmail.com"
                    || (!blob.is_empty()
                        && (host == "www.fastmailusercontent.com"
                            || host
                                .strip_suffix("-www.fastmailusercontent.com")
                                .is_some_and(|region| {
                                    !region.is_empty()
                                        && region.bytes().all(|c| c.is_ascii_alphanumeric())
                                })))
            });
        if (!fastmail && url.origin() != self.origin.origin())
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(JmapError::Protocol);
        }
        Ok(url)
    }
    pub(crate) fn send(&self, request: RequestBuilder) -> Result<Response> {
        checked(
            request
                .bearer_auth(self.token.expose_secret())
                .send()
                .map_err(|e| e.without_url())?,
        )
    }
    pub(crate) fn upload(&self, raw: &[u8]) -> Result<String> {
        let url = self.endpoint(&self.session.upload_url, "")?;
        let value: Value = json_body(
            self.send(
                self.http
                    .post(url)
                    .header("Content-Type", "message/rfc822")
                    .body(raw.to_vec()),
            )?,
        )?;
        if field(&value, "accountId")? != self.account
            || value["size"].as_u64() != Some(raw.len() as u64)
        {
            return Err(JmapError::Protocol);
        }
        Ok(field(&value, "blobId")?.to_owned())
    }
    pub(crate) fn download(&self, blob: &str) -> Result<Vec<u8>> {
        bounded(
            self.send(
                self.http
                    .get(self.endpoint(&self.session.download_url, blob)?),
            )?,
            64 * 1024 * 1024,
        )
    }
    /// Open a one-state EventSource stream. Workers reconnect and sync from their durable cursor.
    pub fn events(&self) -> Result<impl Read> {
        self.send(
            self.http
                .get(self.endpoint(&self.session.event_source_url, "")?)
                .header("Accept", "text/event-stream"),
        )
    }
    pub(crate) fn calls(&self, calls: &[Value], submission: bool) -> Result<Vec<Value>> {
        let mut using = vec![CORE, MAIL];
        if submission {
            using.push(SUBMISSION);
        }
        let body = json!({"using":using,"methodCalls":calls});
        let reply: Value = json_body(
            self.send(
                self.http
                    .post(self.endpoint(&self.session.api_url, "")?)
                    .json(&body),
            )?,
        )?;
        let responses = reply["methodResponses"]
            .as_array()
            .ok_or(JmapError::Protocol)?;
        let mut values = Vec::new();
        for call in calls {
            let r = responses
                .iter()
                .find(|r| r[2] == call[2] && (r[0] == call[0] || r[0] == "error"))
                .ok_or(JmapError::Protocol)?;
            if r[0] == "error" {
                return Err(match field(&r[1], "type")? {
                    "cannotCalculateChanges" => JmapError::StateExpired,
                    "serverFail" | "serverPartialFail" => JmapError::ServerFailure,
                    _ => JmapError::Rejected,
                });
            }
            if field(&r[1], "accountId")? != self.account {
                return Err(JmapError::Protocol);
            }
            values.push(r[1].clone());
        }
        Ok(values)
    }
    pub(crate) fn method(&self, name: &str, mut args: Value) -> Result<Value> {
        args["accountId"] = json!(self.account);
        self.calls(
            &[json!([name, args, "a"])],
            name.starts_with("Identity/") || name.starts_with("EmailSubmission/"),
        )?
        .pop()
        .ok_or(JmapError::Protocol)
    }
}
