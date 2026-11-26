# Fastmail JMAP

In Internet Accounts, choose Fastmail, open Server settings, and select JMAP. Sign in with Fastmail or use an API token with core, mail, and submission access. Accounts saved without a transport continue to use IMAP and SMTP. Reconnect to change transport. Finish pending server operations before switching. Each protocol maintains its own remote message identities.

The initial scan saves immutable MIME once per email and tracks all mailbox memberships. Subsequent syncs request mailbox and email changes together with metadata for created and updated IDs. The EventSource stream is available on the network client for background-worker integration. The app currently receives remote changes on scheduled or manual sync. Local JMAP edits wake the network worker.

## Offline verification

```sh
cargo test --locked -p moonmail-network --test jmap
```

The HTTP fixture uses loopback, synthetic MIME and a disposable SQLite archive.

## Two-account Stalwart test

Run `just test-stalwart` on Apple Silicon macOS or x86-64 Linux. The runner downloads the pinned Stalwart 0.15.5 release, verifies its SHA-256 digest, and starts an HTTP listener on loopback. To reuse a local build, set `STALWART_BIN` to the executable. A temporary SQLite directory holds both synthetic accounts. The runner obtains separate OAuth device grants and removes the server and its data after the test.

Moonmail opens Bob’s EventSource stream before Alice sends from the frozen outbox. After receiving Bob’s Email state change, it reopens the archive, syncs from the saved checkpoint and checks the Message-ID and body in Bob’s Inbox and Alice’s Sent. A second local archive independently verifies read/unread, star/unstar, keyword add/remove, copy, archive, trash, permanent destroy, and mailbox creation, rename and deletion. The Linux protocol workflow runs the same command. Keep the scripted HTTP tests for interrupted transfers, ambiguous submissions and expired state.

## Live round trip

Use two dedicated Fastmail accounts containing only synthetic test mail. Each token needs mail and submission access. Supply the following variables privately through your environment or secret manager.

| Variable | Value |
| --- | --- |
| FASTMAIL_TEST_A_EMAIL | Sender identity |
| FASTMAIL_TEST_A_TOKEN | Sender API token |
| FASTMAIL_TEST_B_EMAIL | Recipient identity |
| FASTMAIL_TEST_B_TOKEN | Recipient API token |

```sh
MOONMAIL_LIVE_FASTMAIL=1 cargo test --locked -p moonmail-network --test fastmail_live -- --ignored --exact fastmail_round_trip
```

The test validates both identities, claims one frozen outbox message, and sends it once. It synchronizes the dedicated accounts into a temporary archive and checks the Message-ID, subject and body in Sent and Inbox. Allow up to five minutes for observation. The temporary archive is removed on exit. The test messages remain in the test accounts for inspection. If submission is uncertain, check the sender’s Drafts and Sent before starting another run.
