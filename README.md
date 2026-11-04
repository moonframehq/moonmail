# Moonmail

A runnable, local-first Rust mail client on Moonframe, built around a separate
MailMate-style **Correspondence** pane. The initial macOS base includes a synthetic
demo, persistent mail, search, Fastmail authentication, IMAP sync, and SMTP sending.

## Start here

Install Rust through rustup, Xcode Command Line Tools, and [Just](https://just.systems).
The repository pins Rust **1.99.0**. The first build compiles Moonframe/GPUI and can
take several minutes.

```sh
just demo                  # launch 72 synthetic messages; no network or credentials
just run                   # open your own archive (initially empty)
just build                 # target/release/moonmail, with embedded app resources
just check                 # headless UI check using a temporary synthetic archive
```

The MailMate-style layout has a full-height mailbox sidebar, a dense message table
across the top, **Correspondence below-left**, and the message reader below-right.
Drag the dividers to resize the panes. Tables use compact, striped rows; the reader
has a small From/Subject/Date/To header and follows the system light/dark appearance.
Select a related message without losing the original correspondence anchor.
The separate reply-thread view groups by the first References/In-Reply-To key.
Use the **thread-list toolbar button** to group the main listing; disclosure
carets expand/collapse replies. Groups are paged as units, with the RFC root
(or earliest matching message) as their representative. Expanded replies obey
the current folder, filter and search; each expansion shows at most 100 messages.
Click **From**, **Subject**, or **Date** to sort; click again to reverse. Date
sorting uses the newest matching member for a thread. Text sorts are ASCII
case-insensitive with stable ID tie-breakers. Correspondence sorts its displayed
bounded result separately.

The sidebar has collapsible mailbox shortcuts and per-account **Sources** trees.
Server folder delimiters and non-selectable structural parents are preserved.
Folder creation/moves are not yet implemented.

**⌘,** or the toolbar gear opens a separate **Settings** window, modeled after
Moonmux, with macOS-style grouped settings: Appearance, Fonts, Reading & Lists,
Internet Accounts, and Storage. The **Fonts** browser lists installed macOS
families through Core Text, supports search, and previews the selected family and
size without saving it. Choose Interface, Message Lists, or Reader, then **Apply**.
The defaults match the
reference MailMate setup: **JetBrains Mono 12** for lists, **System 12** for the
interface, and **Helvetica 12** for the reader. The interface and lists share a
size; the reader has its own. Changes apply live. Threaded-list and mark-as-read
preferences are saved too. System/Light/Dark appearance uses Moonframe's setting.

Preferences live in `preferences.json` beside the archive. Font overrides occupy
a managed block in `~/.config/moonmail/style.luau` (`moonmail-demo` for the demo,
or the `--user` directory); other user styling is preserved. `XDG_CONFIG_HOME`
overrides the config base directory.
⌘N composes, ⌘F focuses search, ⌘R syncs, and ⌘↑/↓ navigates the current page.
Search is submitted with Enter or the Search button; clear the field and submit
to restore the current folder. Drafts save locally as you type.

The sparkle button at the top right opens Moonframe's **Make it yours** dialog:
describe a change or inspect an element, then use an installed ACP coding agent
to edit the app. Development UI changes reload in the running window.

## Fastmail

Use **Settings → Internet Accounts → Add Account…**, enter an email address,
then **Continue**. Moonmail recognizes common provider domains and checks DNS MX
records for custom domains using the system resolver. You can also **choose from
a list**, including Fastmail for custom domains. Only Fastmail sign-in is currently
implemented; other providers are explicitly marked as unsupported. The offline
demo recognizes known domains without making DNS requests.

Choose **Sign In with Fastmail**, authorize in your browser,
then click **Sync**. Alternatively, close the window and run:

```sh
just login you@fastmail.com  # OAuth public-client registration + PKCE
just sync                   # download folders, reconcile UIDs, upload pending flags
just run
```

For a Fastmail app password, use `just password you@fastmail.com`; the password is
entered at a hidden terminal prompt. Use an app password, not your login password.
OAuth credentials and app passwords are stored in macOS Keychain. OAuth supports
loopback callbacks, state/issuer validation, rotating refresh tokens, and revocation
through **Disconnect account** or `just run disconnect`. An interrupted refresh
requires reconnecting rather than risking reuse of a rotated refresh token.

Authentication and synchronization run on a separate connection-owning thread;
local navigation stays available. Sync is manual in this version. Read/star changes
are durable locally and uploaded on the next sync.

Compose accepts comma-separated bare addresses. **Send message** freezes the MIME
bytes and envelope in the durable outbox before making one submission attempt.
Outbox shows queued, sending, sent (accepted by SMTP), failed, or unknown. An
interrupted/ambiguous attempt becomes **unknown**, and is never automatically
resent. Check your server's Sent folder before composing a replacement. SMTP
acceptance does not prove final delivery.

**Live Fastmail OAuth, IMAP, and SMTP interoperability still requires an authenticated
account test.** Development checks use synthetic mail and fake adapters; they do
not access personal archives or send email.

## Local data and import

Default archive: `~/Library/Application Support/moonmail/`.
Demo archive: `~/Library/Application Support/moonmail-demo/`.
Set `MOONMAIL_DATA_DIR` to override the archive path. One process owns each archive;
close the GUI before using CLI commands against the same archive.

```sh
just import /path/to/eml-directory  # recursively read .eml; source files stay untouched
just run status
just reindex
MOONMAIL_DATA_DIR=/path/to/archive just run
```

- `archive.sqlite3`: authoritative metadata, bodies, flags, drafts, outbox, journal.
- `messages/<prefix>/<sha256>.eml`: immutable original bytes, verified on read.
- `search.sqlite3`: rebuildable SQLite FTS5 projection.
- `account.json`: account addresses and server configuration, no tokens/passwords.
- Temporary reader files: sanitized local HTML, removed on normal shutdown.

Back up the **whole archive**, with the app closed. Local mail is not encrypted by
Moonmail; use the operating system's disk encryption as appropriate. Removing an
account connection retains local mail. Expunged/old UIDVALIDITY occurrences leave
live views; original bytes remain in the archive.

## Current limits

- One configured account per archive; macOS Keychain persistence only.
- Manual full UID/flag reconciliation, no IDLE, CONDSTORE/QRESYNC, sync cancellation,
  folder moves/deletes, or background scheduling yet. Folder snapshots keep UID
  metadata in memory, so this is not a demonstrated million-message sync engine.
- Messages above 64 MiB are skipped. Downloads commit individually and resume by
  durable remote identity. Metadata list pages contain at most 75 messages.
- Search uses ANDed literal terms, limited to the first 1,000 ranked candidates
  before folder/filter selection and chronological display. The cap is labeled.
- Correspondence matches sender/recipient pairs in either direction, excludes
  common junk/trash names, and shows at most 100 messages. Reply threading is a
  root-key grouping, not a complete recursive RFC thread graph. The main threaded
  listing uses this same grouping, with one level of expandable replies. Grouped
  queries compute ranks over matching metadata; they are not yet benchmarked on
  million-message archives.
- Plain-text composition only; one resumable draft in the UI. No attachment UI,
  signatures, alias selection, or rich-text composition. Raw MIME attachments are
  preserved. A reply resumes an already open draft rather than overwriting it.
- HTML is sanitized with Ammonia and restrictive CSP; images, scripts, forms and
  links are blocked. Plain text is selectable in the same stable webview.
- No automatic SMTP retries or IMAP Sent-folder append. Outbox retains accepted
  messages and exact submitted bytes; synchronize to see server-side Sent mail.

## Architecture and replaceable implementations

| Crate | Responsibility |
| --- | --- |
| `moonmail-core` | Backend-neutral MIME, blob, repository and search contracts |
| `moonmail-mime` | `mail-parser` and `mailparse` owned extraction adapters |
| `moonmail-store` | SQLite metadata, SHA-256 blobs, journal, flags, drafts/outbox |
| `moonmail-search` | SQLite FTS5, atomic batches, WAL/FULL durable commits |
| `moonmail-network` | Incoming/submission/credential traits, rust-imap, SMTP, OAuth |
| `moonmail-app` | CLI, connection-owning workers, bounded snapshots, Luau UI |
| `moonmail-bench` | Generic synthetic comparisons and conformance fixtures |

IMAP is pinned to **`=3.0.0-alpha.15`**, as selected. Moonframe tracks the **`main`**
branch, with the resolved revision recorded in `Cargo.lock`. SQLite is aligned with Moonframe at
`rusqlite 0.40.2` to avoid duplicate native SQLite linkage.

Adapters preserve typed errors and use static dispatch. Navigation queries keep
bodies separate and use date/ID keyset pagination. Metadata ingestion journals
search changes in the same transaction; the indexer acknowledges them only after
the separate search transaction commits. Interrupted replay is idempotent.

FTS uses `unicode61 remove_diacritics 1`; subject weight 10, sender 4, other fields
1. Literal queries cannot inject FTS syntax. Both MIME adapters normalize decoded
CRLF to LF and retain actual plain/HTML alternatives; raw bytes remain exact.

## Development and benchmarks

Install Taplo and `nightly-2026-06-20` with rustfmt for formatting; builds use the
pinned stable compiler.

```sh
just format
just ci                    # rustfmt/Taplo, Clippy, workspace tests
just bench-check           # exercise all workloads once; no timing claims
just bench                 # Criterion comparisons
just bench mime-owned
just bench index-batch
just bench search-warm
```

For visual layout checks on macOS, use `just screenshot` or
`just screenshot target/screenshots/compose.png compose` (also `settings`,
`outbox`, `threads`, and `feedback`). These launch the real app with isolated synthetic data and export its
GPUI scene, then exit. **Native WebKit reader surfaces are excluded** from that
export and need separate visual checking. Headless checks validate UI trees,
not the final layout; inspect screenshots for clipping and collapsed rows.
The `settings` capture mounts the actual Settings body at its default window
size; `threads` captures an expanded group.
`fonts`, `accounts`, `add-account`, and `providers` capture the font browser and
account setup states. Font previews use native WebKit and are omitted from GPUI
captures, like the message reader.

The generic harness compares identical owned MIME extraction and compatible
search semantics, checking parity outside timed loops. Durable disk commits and
non-durable in-memory runs are separate. Corpus/setup work is outside timings.
Results in `target/criterion/` are synthetic, not real-account or million-message
performance measurements. Reopen tests are not power-loss durability tests.
