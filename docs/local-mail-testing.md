# Local IMAP/SMTP test daemon

`moonmail-test-maild` is an ephemeral Rust fixture for Moonmail's real network
adapters. It uses `imap-codec` for IMAP commands/literals, `smtp-proto` for SMTP,
and `rustls`/`rcgen` for TLS. Tests need no email provider, account secrets, DNS
delivery, or Docker. Building dependencies and images still requires downloads
unless already cached.

## Native tests and interactive use

```sh
just test-mail
# Equivalent:
cargo test --locked -p moonmail-test-maild
cargo test --locked -p moonmail-network --test local_mail

# Keep a daemon running in a terminal:
just test-maild
# Or choose free ports and a separate readiness/certificate directory:
just test-maild --ephemeral-ports --state target/test-maild-manual
```

In-process tests start fresh listeners on ephemeral ports, own the server through
`Server::start(Config)`, and shut down its connections/workers on drop. A standalone
daemon runs until stopped (Ctrl-C). All mail disappears on restart.

| Setting | Value |
| --- | --- |
| Accounts | `alice@moonmail.test`, `bob@moonmail.test` |
| Password for both | `moonmail-test-only` |
| Host | `localhost` (default bind: `127.0.0.1`) |
| IMAP implicit TLS | `1993` |
| SMTP implicit TLS | `1465` |
| IMAP STARTTLS | `1143` |
| SMTP STARTTLS | `1587` |
| Folders per account | INBOX, Sent, Archive, Trash, Junk, Drafts |

Each startup writes `ca.der`, `ca.pem`, and finally `ready.json` under
`target/test-maild` (override with `--state`). The JSON contains the four actual
ports. Readiness is published atomically after binding all listeners; it is not a
health probe after process exit. TLS keys stay in memory. Certificates cover
`localhost`, `127.0.0.1`, and `::1`; leaf certificates last seven days.

The test clients explicitly trust that startup's CA through
`RustImap::connect_with_tls`, `SmtpSubmission::submit_with_tls`, and
`check_connection_with_tls`. They keep certificate and hostname verification.
They do not modify system trust or Keychain. The normal application still uses
system trust; this fixture is primarily for automated adapter tests.

To test a separately running native daemon with client-managed Sent:

```sh
MOONMAIL_TEST_MAILD_STATE=target/test-maild cargo test --locked \
  -p moonmail-network --test local_mail local_round_trip -- --exact --nocapture
```

Supplying this variable requires valid readiness/CA files and a reachable daemon;
a failure does not fall back to an in-process fixture. Use a fresh daemon for each
test campaign to avoid filling its small quotas.

## Docker

```sh
just test-mail-docker
```

This builds `moonmail-test-maild:local` with `docker/test-maild.Dockerfile`, starts
it as an unprivileged user with a read-only filesystem and temporary `/state`,
publishes random loopback ports, waits for readiness, copies the public CA, runs
the same real-client round trip, and removes the container even on failure.

To run the image manually:

```sh
just test-mail-image
docker run --rm --name moonmail-fixture \
  -p 127.0.0.1:1993:1993 -p 127.0.0.1:1465:1465 \
  -p 127.0.0.1:1143:1143 -p 127.0.0.1:1587:1587 \
  moonmail-test-maild:local
```

In another terminal, copy its public connection material and run the command above:

```sh
mkdir -p target/docker-maild
docker cp moonmail-fixture:/state/ca.der target/docker-maild/ca.der
docker cp moonmail-fixture:/state/ready.json target/docker-maild/ready.json
MOONMAIL_TEST_MAILD_STATE=target/docker-maild cargo test --locked \
  -p moonmail-network --test local_mail local_round_trip -- --exact
```

## CI and coverage

`.github/workflows/local-mail.yml` runs on pushes, pull requests, and manual runs:

- Native daemon/network tests on Linux and macOS.
- Build the Docker image on Linux, then exercise both TLS modes against it.
- No mail-provider secrets or external mail services. Images are built locally
  in the job, not published to a registry.

The normal workspace test run also includes the native tests. Coverage includes
two-way SMTP delivery, UTF-8 body and binary attachment preservation, application
reply construction, durable Outbox, Sent APPEND/reconciliation, server-created
Sent copies, reopened archives/connections, account isolation, trust rejection,
wrong passwords, plaintext-auth rejection, rejected relay, flags, read-only
EXAMINE, stable UIDs, MOVE/targeted expunge, atomic quota failures, and shutdown.

The default server does **not** create SMTP Sent copies: Moonmail saves them with
APPEND. `--server-sent` enables provider-style Sent copies; the separate native
`server_managed_sent_round_trip` test exercises that policy.

## Deliberately limited protocol fixture

This is not a general-purpose mail server or a full IMAP conformance oracle:

- Fixed two-account namespace; SMTP can only send as the authenticated account
  and deliver to these accounts. LOGIN and AUTH PLAIN require TLS. No relay.
- IMAP supports CAPABILITY, STARTTLS, LOGIN, LIST with empty reference and `*` or
  `%`, SELECT/EXAMINE, NOOP/CHECK/LOGOUT, APPEND with synchronizing literals,
  UID FETCH/STORE, single-message UID COPY/MOVE, and UID EXPUNGE.
- FETCH supports UID, FLAGS, RFC822.SIZE, INTERNALDATE, and full BODY[]/BODY.PEEK[].
  At most 16 requested items and one body item. Sequence-number operations,
  sections, partial bodies, macros, and unsupported items are rejected.
- Fixed synthetic INTERNALDATE (`01-Jan-2026`); explicit APPEND dates are rejected.
  No CREATE/RENAME/DELETE, SEARCH, IDLE, OAuth, CONDSTORE/QRESYNC, ordinary EXPUNGE,
  remote drafts, subscriptions, or unsolicited cross-session change notifications.
  Re-select/re-scan to observe other sessions' changes.
- Valid unsupported commands return NO; malformed/oversized commands and
  unsupported literal modes close the connection. It does not advertise LITERAL+.
- Limits: 1 MiB per message, 32 MiB stored message payloads, 256 messages per
  mailbox, 32 concurrent sessions, 10-second socket inactivity timeouts, bounded
  command lines/literals and flag collections. Payload quota is not a total
  process-RSS limit. COPY/MOVE may need temporary capacity for another copy.
- No persistence, external delivery, spam filtering, or provider-specific Gmail
  label behavior. Restart generates a new CA and randomized UIDVALIDITY; UIDs
  increase monotonically within each mailbox during a run.

Keep [live Gmail testing](gmail-testing.md) as a separate optional interoperability
check; this fixture cannot establish Gmail or other provider compatibility.
