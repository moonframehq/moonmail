//! A single EventSource listener wakes the account worker. Dropping its interest
//! cancels future requests and prevents a late response waking a paused account.
use {
    crate::jmap::Client,
    std::{
        io::{BufRead, BufReader, Read},
        sync::Arc,
        thread,
        time::Duration,
    },
};

pub(super) fn listen(
    client: &Arc<Client>,
    wake: impl Fn() + Send + 'static,
) -> std::io::Result<Arc<()>> {
    let interest = Arc::new(());
    let active = Arc::downgrade(&interest);
    let client = Arc::downgrade(client);
    thread::Builder::new()
        .name("moonmail-jmap-events".into())
        .spawn(move || {
            while active.strong_count() > 0 {
                let Some(client) = client.upgrade() else {
                    break;
                };
                // A timeout or an interrupted stream reconnects. The periodic sync
                // on the owning worker covers gaps between event subscriptions.
                let changed = client.events().ok().is_some_and(|response| {
                    // Catch changes between the last checkpoint and subscription, including reconnects.
                    if active.upgrade().is_some() {
                        wake();
                    }
                    state_changed(response, &client.account)
                });
                if active.strong_count() == 0 {
                    break;
                }
                if changed {
                    wake();
                }
                thread::sleep(Duration::from_secs(if changed { 1 } else { 5 }));
            }
        })?;
    Ok(interest)
}

fn state_changed(stream: impl Read, account: &str) -> bool {
    let reader = BufReader::new(stream.take(65536));
    let mut data = String::new();
    for line in reader.lines() {
        let Ok(line) = line else { return false };
        if line.is_empty() {
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(&data)
                && event["@type"] == "StateChange"
                && let Some(types) = event["changed"][account].as_object()
                && (types.contains_key("Email") || types.contains_key("Mailbox"))
            {
                return true;
            }
            data.clear();
        } else if let Some(value) = line.strip_prefix("data:") {
            data.push_str(value.trim_start());
            data.push('\n');
        }
    }
    false
}

#[cfg(test)]
mod tests;
