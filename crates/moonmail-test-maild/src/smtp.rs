use {
    crate::{
        Result,
        server::Shared,
        store::{ACCOUNTS, MAX_MESSAGE, PASSWORD},
        wire::Wire,
    },
    base64::{Engine, engine::general_purpose::STANDARD},
    smtp_proto::{AUTH_PLAIN, Request},
    std::sync::Arc,
};

pub(crate) fn serve(wire: &mut Wire, shared: &Shared) -> Result {
    wire.send("220 localhost Moonmail test SMTP\r\n")?;
    let mut greeted = false;
    let mut account = None::<String>;
    let mut sender = None::<String>;
    let mut recipients = Vec::new();
    loop {
        let line = wire.line(8192)?;
        let request = match Request::parse(&mut line.iter()) {
            Ok(request) => request,
            Err(_) => {
                wire.send("500 Invalid command\r\n")?;
                continue;
            },
        };
        match request {
            Request::Ehlo { .. } | Request::Helo { .. } => {
                greeted = true;
                sender = None;
                recipients.clear();
                wire.send(if wire.encrypted {
                    "250-localhost\r\n250-AUTH PLAIN\r\n250 SIZE 1048576\r\n"
                } else {
                    "250-localhost\r\n250 STARTTLS\r\n"
                })?;
            },
            Request::StartTls if !wire.encrypted && greeted && account.is_none() => {
                wire.send("220 Begin TLS\r\n")?;
                wire.upgrade(Arc::clone(&shared.tls))?;
                greeted = false;
                sender = None;
                recipients.clear();
            },
            Request::Auth {
                mechanism: AUTH_PLAIN,
                initial_response,
            } if wire.encrypted && greeted && account.is_none() => {
                let encoded = if initial_response.is_empty() {
                    wire.send("334 \r\n")?;
                    wire.line(8192)?
                } else {
                    initial_response.as_bytes().to_vec()
                };
                let decoded = STANDARD
                    .decode(encoded.strip_suffix(b"\r\n").unwrap_or(&encoded))
                    .unwrap_or_default();
                let parts: Vec<_> = decoded.split(|b| *b == 0).collect();
                if let [authz, user, password] = parts.as_slice()
                    && (*authz == b"" || authz == user)
                    && *password == PASSWORD.as_bytes()
                    && let Ok(user) = std::str::from_utf8(user)
                    && ACCOUNTS.contains(&user)
                {
                    account = Some(user.into());
                    wire.send("235 Authenticated\r\n")?;
                } else {
                    wire.send("535 Authentication failed\r\n")?;
                }
            },
            Request::Mail { from } if wire.encrypted && greeted && account.is_some() => {
                sender = None;
                recipients.clear();
                if account.as_deref() == Some(from.address.as_ref()) {
                    sender = Some(from.address.into_owned());
                    wire.send("250 Sender accepted\r\n")?;
                } else {
                    wire.send("550 Sender must be the authenticated fixture account\r\n")?;
                }
            },
            Request::Rcpt { to } if sender.is_some() => {
                if ACCOUNTS.contains(&to.address.as_ref()) && recipients.len() < 2 {
                    recipients.push(to.address.into_owned());
                    wire.send("250 Recipient accepted\r\n")?;
                } else {
                    wire.send("550 No external delivery\r\n")?;
                }
            },
            Request::Data if sender.is_some() && !recipients.is_empty() => {
                wire.send("354 End with dot\r\n")?;
                let mut raw = Vec::new();
                loop {
                    let line = wire.line(MAX_MESSAGE)?;
                    if line == b".\r\n" {
                        break;
                    }
                    raw.extend_from_slice(line.strip_prefix(b".").unwrap_or(&line));
                    if raw.len() > MAX_MESSAGE {
                        wire.send("552 Message too large\r\n")?;
                        return Ok(());
                    }
                }
                let result = shared.store()?.deliver(
                    sender.as_deref().unwrap_or_default(),
                    &recipients,
                    &raw,
                );
                sender = None;
                recipients.clear();
                wire.send(if result.is_ok() {
                    "250 Delivered locally\r\n"
                } else {
                    "552 Fixture storage limit\r\n"
                })?;
            },
            Request::Rset => {
                sender = None;
                recipients.clear();
                wire.send("250 Reset\r\n")?;
            },
            Request::Noop { .. } => wire.send("250 OK\r\n")?,
            Request::Quit => {
                wire.send("221 Bye\r\n")?;
                return Ok(());
            },
            _ => wire.send(
                "503 Unsupported command or wrong state; TLS and authentication required\r\n",
            )?,
        }
    }
}
