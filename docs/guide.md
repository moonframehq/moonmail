# Moonmail guide

The full reference: every feature, its limits, and how to develop Moonmail.

Moonmail is a runnable, local-first Rust mail client on [Moonframe](https://moonframe.dev), built around a separate
MailMate-style **Correspondence** pane. The initial macOS base includes a synthetic
demo, persistent mail, search, multi-provider authentication, IMAP sync, and SMTP sending.

## Start here

Install Rust through rustup, Xcode Command Line Tools, and [Just](https://just.systems).
The repository pins Rust **1.99.0**. The first build compiles Moonframe/GPUI and can
take several minutes.

```sh
just demo                  # launch 72 synthetic messages; no network or credentials
just run                   # open your own archive (initially empty)
just debug                 # same, with account logs in target/moonmail-debug.log
just build                 # target/release/moonmail, with embedded app resources
just check                 # headless UI check using a temporary synthetic archive
```

When browser sign-in returns to Moonmail and setup stops, run `just debug` and retry Add Account.
The Fastmail trace in `target/moonmail-debug.log` shows the callback, token exchange,
IMAP and SMTP checks, and authentication rejection codes. These events omit account
addresses, tokens, and raw server replies.

The MailMate-style layout has a full-height mailbox sidebar, a dense message table
across the top, **Correspondence below-left**, and the message reader below-right.
Drag the dividers to resize the panes. Tables use compact, striped rows; the reader
has a small From/Subject/Date/To header and follows the system light/dark appearance.
Select a related message without losing the original correspondence anchor.
The separate reply-thread view follows account-scoped References/In-Reply-To graphs,
including missing ancestors and parents arriving after their replies.
Use the **thread-list toolbar button** to group the main listing; disclosure
carets expand/collapse replies. Groups are paged as units, preferring matching
messages without parent links, then earliest date/local ID. Expanded replies obey
the current folder, filter and search; each expansion shows at most 100 messages.
Click **From**, **Subject**, or **Date** to sort; click again to reverse. Date
sorting uses the newest matching member for a thread. Text sorts are ASCII
case-insensitive with stable ID tie-breakers. Correspondence sorts its displayed
bounded result separately.

Cycles terminate safely. Conflicting copies of a Message-ID lose their automatic
ID links; identical-byte copies can share a conversation. Explicit reference links
remain. Thread processing retains at most 256 references (oldest plus nearest 255)
and reports omitted/malformed links in the reader; original headers stay intact.
Existing archives migrate metadata in resumable 100-message transactions on open;
opening waits for completion. Expanded presentation remains a flat chronological
list, rather than a recursive parent/child tree.

The sidebar has collapsible mailbox shortcuts and per-account **Sources** trees.
Server folder delimiters and non-selectable structural parents are preserved.
After a complete successful server folder list, vanished folders leave navigation
and their remote occurrences/pending flags retire together. Original mail remains
archived; failed folder lists do not retire anything.
**Mail → Manage folders…** creates folders, renames a subtree, or deletes a named
leaf and its server contents. Deletion requires typing its full name; downloaded
original bytes remain archived. INBOX is protected. Remote changes queue for Sync,
revalidate namespace membership and UIDVALIDITY, and preserve mailbox IDs and role
overrides on acknowledged rename. Inspect uncertain results in **Mail operations**;
they never retry automatically. Pending folder edits and transfers exclude one
another within an account. Local/demo folder edits apply immediately.

Use the **Archive**, **Trash**, or **Move & folders** controls to file mail. Check
up to 100 individual message rows for a bulk move; a collapsed thread row selects
only its representative, not unseen replies. Changing folder/search/page clears
the checked selection. Moves are account-scoped; ambiguous source memberships
require choosing the source folder first.

**Move & folders** also offers explicit read/unread and flag/unflag actions for
the selected messages. Flags update every live occurrence and queue remote intent
atomically; a pending transfer on any selected message rejects the entire batch.
These actions affect checked representatives, not hidden thread replies.

**Mail → Tags & keywords…** adds/removes an IMAP keyword on up to 100 selected
messages, across their live folder occurrences. Wire names are case-insensitive
ASCII atoms (up to 255 bytes); Unicode friendly labels are account-local mappings.
Renaming a label does not rename the server keyword. The reader selection's keyword
union and the first 500 account labels appear in the panel.

Keyword edits and remote intent commit atomically. Complete sync inventories publish
server membership while preserving pending local edits; failed scans cannot clear
tags. Keyword STORE validates UIDVALIDITY, UID and explicit PERMANENTFLAGS support.
Lost acknowledgments retry idempotent add/remove on the next Sync. Unsupported or
stale operations remain visible, with paged **Retry** / **Discard intent**
controls; discarding restores observed state on the next complete Sync. Absent or
empty PERMANENTFLAGS are conservatively unsupported because the pinned adapter
does not distinguish them. Incoming observations and local additions are bounded
to 128 keywords per occurrence. Transfers wait for earlier keyword intent; blocked
edits fail a queued transfer before dispatch. Cancel that failed transfer and queue
it again after resolving the keyword operation.

**Mail → Folder-entry rules…** configures up to 100 ordered keyword rules for
physical folders. New rules start disabled in the editor. Conditions accept
metadata fields (`account:`, `in:`, `is:`, `keyword:`, `tag:`, `sender-address:`,
`recipient-address:`, `after:`, `before:`)
and Boolean operators; an empty condition matches every new entry. Text/body
conditions, smart-mailbox entry triggers, and move actions remain unfinished.

An enabled rule runs when a new folder occurrence is ingested or created by a local
copy/move. Rules run in creation order; later rules see earlier keyword changes.
The action, remote keyword intent, and matching-run history commit with the folder
entry. Restart, reindex, repeated sync, and copying to an existing occurrence do
not replay rules. Actual exit/reentry and a replacement UIDVALIDITY namespace are
new entries. Saving, enabling, or editing a rule leaves existing members alone.
**Apply to selected** explicitly evaluates the current version against up to 100
selected source-folder members, including disabled rules. Actions affect every live
occurrence of each matching message, just like manual keyword edits. Pending
transfers or keyword limits produce blocked history entries; resolve the cause and
apply explicitly. Storage failures roll back entry and action together. History
pages 100 matching runs at a time and retains old names/revisions after rule deletion.
When ingest queues a rule edit, the account worker runs another write pass.

Server SPECIAL-USE roles drive shortcuts and Archive/Trash destinations, including
localized folder names. **Move & folders** lets you assign Archive/Trash overrides
or restore server defaults. Missing/ambiguous roles require an explicit choice;
a missing overridden folder never silently redirects to a different destination.

Remote moves persist locally and wake the account worker, which drains queued transfers in batches of up to 100.
Queued messages remain in the source view until the server acknowledges the move;
destination messages are refreshed by the same sync. **Mail operations** shows
queued, running, done, failed, and uncertain outcomes, with cancellation before
dispatch. A crash or lost reply after dispatch becomes **unknown** and is never
automatically retried: sync, inspect both folders, then dismiss the record.
Existing read/star intent is applied before moving; new flag edits wait until the
move finishes or is cancelled. Local demo/import moves apply immediately.

**Copy here** and **Mail → Copy to mailbox…** use the same account/source checks and
durable journal, retaining the source. Standalone COPY works without UIDPLUS;
acknowledged copies without a UID mapping complete and are discovered by ordinary
sync. Lost acknowledgments remain unknown and never repeat automatically. Repeated
local copies preserve an existing destination occurrence and its flags.

Servers without MOVE can use a UIDPLUS fallback: COPY one UID, durably retain its
destination UID mapping, verify that destination's exact bytes against the archive,
revalidate the source UIDVALIDITY, then delete and UID EXPUNGE only that source UID.
Ordinary EXPUNGE is never used. Missing mapping, mismatched bytes/epochs, interrupted
copy, and uncertain removal preserve an **unknown** operation for inspection.
Cancellation after copying retains the source; uncertain copies are never repeated
automatically. Verification is bounded to 64 MiB. Live-server interoperability is
still unverified.

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

**Settings → Reading & Lists → Message columns** controls main-list column order,
visibility, and widths. From, Date, and To can be hidden; Subject stays visible
and fills the remaining space. To displays recipients without sorting. Fixed
widths range from 80–300 points, with at most 700 points visible in total. Thread
disclosure moves to Subject when From is hidden. Changes apply immediately and
survive restart; Correspondence keeps its compact Subject/Date layout.

Preferences live in `preferences.json` beside the archive. Font overrides occupy
a managed block in `~/.config/moonmail/style.luau` (`moonmail-demo` for the demo,
or the `--user` directory); other user styling is preserved. `XDG_CONFIG_HOME`
overrides the config base directory.
⌘N composes, ⌘F focuses search, ⌘R syncs, and ⌘↑/↓ navigates messages across
main-list page boundaries. First / Previous / Next page controls preserve the
current folder, filter, search, grouping and sort order.
Search is submitted with Enter or the Search button; clear the field and submit
to restore the current folder. Drafts save locally as you type.

Search accepts literal words/quoted phrases, fields (`subject:`, `body:`, `from:`,
`to:`, `cc:`), exact `account:` and `in:` mailbox names, `is:unread`, `is:read`,
`is:starred`, `keyword:` wire names, exact account-local `tag:` labels, and
`after:` / `before:` calendar dates. Uppercase `NOT`, `AND`, `OR`
and parentheses compose conditions; adjacent terms mean AND. Precedence is NOT,
then AND, then OR. Date boundaries are UTC midnight: after is inclusive, before
exclusive. Text fields use FTS token matching, not exact email-address equality.
Use `sender-address:` for exact parsed From addresses or `recipient-address:` for
exact To/Cc addresses. These preserve local-part case and fold ASCII domain case,
matching Correspondence; they do not infer Bcc recipients.

```text
(from:clara OR cc:julian) subject:"design review" NOT is:read
in:INBOX after:2026-01-01 before:2027-01-01
keyword:project-work OR tag:"Follow up"
```

Quote literal operator words or text containing a colon, such as `"OR"` or
`"https://example.org"`. Invalid syntax reports an error and retains the previous
query. Limits are 4 KiB input, 64 conditions and 16 nested levels.
**Saved searches** stores up to 100 named queries; saving the same name replaces
it. Opening evaluates current mail across all folders and clears toolbar flag
filters. Include `in:` / `account:` to retain a scope in the saved query.
Use **Use source** to select other saved searches before saving: their results
are combined with OR, then filtered by the current query. Sources can themselves
have sources. **Edit** reloads the saved name, query and source selections. Parent
edits affect descendants on refresh; cyclic dependencies, missing sources and
combined queries over the search limits are rejected atomically. Remove source
dependencies before deleting a parent. Submitting the toolbar query directly
leaves the saved-source context.

**Saved searches → Partitions** automatically groups the complete resolved query
by sender address, To/Cc address, or IMAP keyword. The field choice persists with
that saved search. Groups count distinct live messages and unread messages before
paging; multiple addresses/keywords can put a message in several groups. Empty
values are omitted. First/Next controls cover all groups in 100-value pages.
**Open group** intersects the saved predicate with the exact group value, retaining
that filter across navigation, refresh and parent-search edits. Changing the field
or opening the parent search clears the group filter. These are a paged group view;
automatic sidebar subfolders and virtual-membership rule triggers remain future work.

**⌘L** opens the mailbox chooser; **⌘⇧M** opens the move chooser and **⌘⌥M** the
copy chooser. Type words from
the folder path or account address, use ↑/↓ to select, then Enter. Escape cancels.
At most 100 matching folders are shown; keep typing to narrow. Move destinations
are restricted to the selected messages' account, and a selection/source-folder
change invalidates a stale move action. **⌘⇧A** archives the selection; **⌘⇧U**
marks the current message unread.

**⌘⌥↑/↓** seeks unread messages in the current folder/search/flag scope without
wrapping. In grouped mode it advances to the previous/next unread conversation,
expands it, and selects its earliest matching unread member, even beyond the first
100 replies. **⌘⇧↑/↓** moves between threads and enables grouped view. The Mail menu
also offers **Mark entire thread read/unread**: this affects all live members across
folders in that account, independent of the displayed page or search filter. A
pending/uncertain move on any member rejects the entire flag change atomically.

**Save .eml…** above the reader exports the original, hash-verified message bytes
through the native save panel. The filename includes its local message ID.
`moonmail export MESSAGE_ID DESTINATION.eml` provides the same export from the CLI.
Existing files are never replaced; MIME headers, attachments and line endings
are preserved exactly. Importing the exported `.eml` recovers those original bytes.

**Mail → Export selected originals…** exports up to 100 selected messages to a new
private subfolder of a chosen directory. Each `.eml` preserves exact original bytes;
`manifest.json` records IDs, filenames, SHA-256 hashes and sizes and is written last.
`moonmail export-many DIRECTORY ID [ID ...]` does the same from the CLI. Repeated
exports create new folders. Ordinary errors remove the incomplete new export;
after a crash, a folder without a manifest is incomplete. Import the exported
folder to recover message contents. This export does not include folder membership,
flags, drafts, settings, or credentials.

Each **Compose**, **Reply**, or **Reply All** opens an independent saved draft in
a separate, resizable native composer window. The mailbox remains usable: read,
search, or navigate other messages while writing. The focused composer has compact
address rows, optional Cc/Bcc fields, a borderless writing surface, and Send at the
top. **Save & close** retains the draft; sending or discarding closes its window
only after the operation succeeds. The pinned window API provides one reusable
composer window; opening another draft replaces its contents after earlier edits
have been saved. **Drafts** uses the same dense message-table styling as the inbox,
paging 50 metadata rows at a time, newest edits first. Click a row to reopen it;
discard is available in the composer. Opening loads its body and private recipients.
Failed saves leave the last saved version active and report an error.

Use **Attachments…** above the reader to inspect files on demand, including inline
images and attached messages. **Save As…** opens the native save panel and writes
the transfer-decoded bytes unchanged. Suggested filenames are reduced to safe
basenames. Choose a new destination: existing files are never overwritten.

In a draft, the paperclip opens the native multi-file picker. Files are copied
into immutable local storage before the draft is updated, so later source-file
edits/deletion do not change the message. Use the attachment's × to detach it. Drafts
support up to 16 files and 25 MiB total before MIME encoding. Sending verifies
every blob and embeds the files in the frozen MIME; missing/corrupt files fail
preparation without queuing a partial message.

The sparkle button at the top right opens Moonframe's **Make it yours** dialog:
describe a change or inspect an element, then use an installed ACP coding agent
to edit the app. Development UI changes reload in the running window.

## Connect email accounts

Use **Settings → Internet Accounts → Add Account…**, enter an email address,
then **Continue**. Moonmail recognizes common provider domains and checks DNS MX
records for custom domains using the system resolver. You can also **choose from
a list** for custom domains. Presets cover Fastmail, Gmail/Google Workspace,
Microsoft 365/Outlook, iCloud, Yahoo, and AOL. **Other Mail Account** accepts your
IMAP/SMTP hosts, ports, separate login usernames, and implicit TLS or required
STARTTLS. Both services must authenticate before an account is registered.
**Check** verifies a connected account without sending mail; **Reconnect…** updates
its credentials/settings. The offline demo never connects.

### Gmail and Google Workspace

Select **Google · Gmail**, then **Use app password…** if your Google account offers
app passwords (normally requires 2-Step Verification and may be restricted by your
organization). The native secure-entry dialog keeps the password out of UI state.
If your build supplies a registered Google desktop client, choose **Sign in with Google**.
For browser sign-in using your own client:

1. Create your own Google Cloud OAuth client with application type **Desktop app**.
   Configure the consent screen and allow the Gmail scope `https://mail.google.com/`.
   Add your account as a test user if the application is in testing.
2. Download its client JSON and choose **Import OAuth client…** in Moonmail.
3. Choose **Sign in with Google**, authorize the same email address, then **Sync**.

Imported clients and account tokens are stored in Keychain. On macOS, `just bundle /path/to/desktop-client.json` builds the release app, copies the client to
`target/bundle/Moonmail.app/Contents/Resources/google-desktop-client.json`, and signs it ad hoc for local use.
Omit the path to build the app without a Google client. Apply your distribution signature to the configured app before archiving it.
A Keychain import takes precedence. The source checkout includes no registration. Google's testing-mode grants can expire after seven days; broader
distribution of this restricted mail scope requires Google's verification process.
See [Google desktop OAuth](https://developers.google.com/identity/protocols/oauth2/native-app)
and [Gmail XOAUTH2](https://developers.google.com/workspace/gmail/imap/xoauth2-protocol).

Gmail labels exposed through IMAP appear as folders. Advertised `\All` maps to
Archive even with localized names. Gmail saves its own Sent copies by default.
Drafts stay local until submission.

### Folder sync and Gmail labels

Move & folders → Manage folders lists every discovered folder. Pause sync to keep a folder in
the tree with its last local contents. Enable sync to refresh it on the next
run. Newly discovered Gmail Important folders start paused. All Mail starts
enabled so archived mail remains available.

Gmail inventory requests the message ID, thread ID and labels together. A
message already downloaded through one folder can appear in another without a
second body download. In Tags & keywords, the Gmail controls edit server labels
on the selected messages. Pending operations survive restart and run on Sync.
Gmail Archive removes the Inbox label unless an explicit Archive folder override
is configured.

### Microsoft, iCloud, Yahoo, AOL, and custom servers

Microsoft browser sign-in uses a user-owned Entra desktop/public client. Register
**Mobile and desktop applications** with redirect URI `http://localhost/`, support
the account types you need (organizational and personal accounts for Outlook.com),
and configure delegated Exchange Online `IMAP.AccessAsUser.All`, `SMTP.Send`, and
offline access. Import a JSON file containing `{"client_id":"YOUR-APPLICATION-UUID"}`;
no client secret is needed. The callback uses a dynamically allocated localhost
port. The mailbox must allow IMAP and SMTP AUTH; tenant policy may require admin
consent. Outlook.com users must enable IMAP in web settings. Common personal domains
use `smtp-mail.outlook.com:587`; organizational accounts use `smtp.office365.com:587`.
This is IMAP/SMTP support, not Exchange/EWS or Microsoft Graph.
See [Microsoft mail OAuth](https://learn.microsoft.com/en-us/exchange/client-developer/legacy-protocols/how-to-authenticate-an-imap-pop-smtp-application-by-using-oauth).

iCloud, Yahoo, and AOL use their provider-issued app passwords. Custom servers use
**Connect with password…**; leave a username blank to use the account email.
Both services currently share one password. Plaintext and unverified TLS are not
offered. OAuth accounts use their provider's fixed preset endpoints and usernames.

CLI equivalents (close the GUI first):

```sh
just run oauth-client google /path/to/desktop-client.json
just run login you@gmail.com google
just run password you@icloud.com icloud
just run connect /path/to/account.json
```

Custom `account.json` example (no password in this file):

```json
{"email":"you@example.org","provider":"other",
 "imap_host":"imap.example.org","imap_port":993,"imap_tls":"implicit","imap_username":"you@example.org",
 "smtp_host":"smtp.example.org","smtp_port":587,"smtp_tls":"starttls","smtp_username":"you@example.org",
 "sent_copy":"append"}
```

### Fastmail and credential lifecycle

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
through **Disconnect** beside an account or `just run disconnect EMAIL` (omit
EMAIL for the first connected account). An interrupted refresh
requires reconnecting rather than risking reuse of a rotated refresh token.
Google refresh preserves an existing refresh token when the response omits one;
Microsoft rotations are persisted. Microsoft Disconnect removes the local grant;
manage server-side application consent in your Microsoft account separately.
Authentication operations share a per-Keychain-account lock across archives and
processes. Contending operations fail before reading or changing credentials;
retry after the other operation completes. Lock files live in
`~/Library/Application Support/moonmail/credential-locks/`, independently of
`MOONMAIL_DATA_DIR`, and remain in place after release.

Each connected account has a worker that owns its authenticated IMAP session.
INBOX changes wake the worker through IDLE. Full folder checks run every 15 minutes
by default and provide polling for servers without IDLE. Local flag, keyword,
transfer and folder edits wake the owning worker after the local commit.

Compose opens an independent window so you can keep reading other mail. One
reusable composer is supported; opening another draft replaces its saved contents.
Drafts appear as compact message-style rows. **Save & close** keeps the draft;
successful queueing or discarding closes the composer.

Sync uploads local edits and imports other devices’ drafts when the account has
one selectable Drafts role. Choose **Use as Drafts** in **Move & folders** to
resolve an ambiguous role. Remote drafts open in the plain-text composer.
Bcc and attachments are part of the server draft. Concurrent edits can produce
two drafts, preserving both versions. If APPEND loses its acknowledgement, Sync
looks for the upload’s unique Message-ID. Check the server before manually
retrying an unknown upload in **Mail operations**.

In **Move & folders**, select **Delete Permanently** for the selected Trash or
Junk message, or **Empty Trash** / **Empty Junk** for the last synced inventory.
Confirm the account and folder. Sync deletes only the confirmed UIDs, preserving
later arrivals and locally archived MIME. Deletion requires UIDPLUS. Check
**Mail operations** for unsupported servers, changed UIDVALIDITY, or interrupted
commands. Retrying a deletion verifies the original UIDVALIDITY again.

Compose has **To**, **Cc**, and **Bcc** fields accepting display names, quoted
commas, and address groups. Unicode display names are supported; mailbox addresses
must be ASCII. At most 100 unique recipients can be sent, deduplicated To → Cc → Bcc
by exact local part and case-insensitive domain. Bcc-only messages are supported;
Bcc recipients live in the local draft and delivery envelope, never delivered MIME.
**Send message** freezes the MIME
bytes and envelope in the durable outbox before making one submission attempt.
Outbox shows queued, sending, sent (accepted by SMTP), failed, or unknown. An
interrupted/ambiguous attempt becomes **unknown**, and is never automatically
resent. Check your server's Sent folder before composing a replacement. SMTP
acceptance does not prove final delivery.

**Queue only** freezes a message without starting a network operation. Outbox
pages all records and lets you send queued messages, cancel queued/failed entries,
or **Return to drafts**. Returning creates an independent draft with the original
To/Cc/Bcc, body, reply ancestry, account, and attachment references, and atomically
cancels the original. Older outbox entries without a compose snapshot cannot be
restored. **Retry submission** is available only after a proven pre-submission
failure; it reuses the same frozen bytes and envelope. Sending, accepted,
cancelled, and unknown outcomes cannot be retried or returned to drafts.

**Forward** creates an independent draft with empty recipients, public original
headers and quoted text, and copied attachments. It starts a new conversation
without reply ancestry or hidden recipients. **Mail → Forward text only** omits
files when desired or when the original exceeds the draft's attachment limits.
Forwarded text is capped at 64,000 characters with an explicit truncation notice.

Connect up to 32 accounts in **Settings → Internet Accounts**. The first connected
account is the default for new messages. The compact **From** menu lists connected
accounts and their configured aliases, with checkmarks, arrow/Enter navigation,
Escape dismissal, and **Manage accounts…**. Selecting a row saves the sending
account and alias together; its account supplies the credentials and SMTP settings.
To/Cc/Bcc, edited body/signature text, attachments, and reply ancestry are preserved.
Accounts sync independently and report their own connection errors. Disconnecting keeps local mail, drafts, and frozen outbox
entries. Sending requires their original account to be reconnected; drafts can
instead choose a different From account. Legacy blank-account drafts require an
explicit From selection. No default-account substitution occurs for outbox items.

**Mail → Identities & signatures** manages up to 32 sending identities per account:
display name, address, optional Reply-To, and a plain-text signature (16 KiB).
Choose an account to edit, then **Make default** for new messages, or the **From**
row in a draft to select a sender explicitly. Replies prefer a configured identity addressed in To/Cc (or
the original From when replying to your own sent mail) and exclude all configured
owned addresses from Reply-All. Domain case is folded; local-part case, plus
suffixes, and dots are preserved. Configure aliases authorized by your provider.
Credentials always belong to the account, while the chosen address is used for
both From and the SMTP envelope sender. Drafts freeze the identity; changing or
removing it in settings does not rewrite saved drafts. A changed or deleted alias
is not checked as though it still matched the frozen draft. A signature is inserted
once when creating a draft. Switching identity preserves your edited body; use
**Insert signature** to explicitly insert the newly selected signature.

Replies retain the parent's References (falling back to its In-Reply-To) across
draft saves and append the immediate parent once at submission. Outgoing chains
retain at most 100 distinct IDs, preserving the root and newest ancestors.
Malformed historical IDs are omitted; a malformed immediate parent is rejected.
Existing archives recover missing parent headers lazily from original MIME.
Replies prefer Reply-To over From; Reply All also includes original To/Cc while
excluding the known account address. Replies to your own sent messages target
original recipients. Hidden Bcc recipients are never copied into replies. Quoted
plain text is capped at 64,000 characters with a visible truncation marker.
Drafts remember their account, and sending through a different account is rejected.

**Live provider OAuth, IMAP, and SMTP interoperability still requires authenticated
account tests.** Development checks use synthetic mail and fake adapters; they do
not access personal archives or send email.

For a live check, connect in **Internet Accounts**, run **Check**, then **Sync**.
Open an Inbox message, verify its body/attachments, and reply to a test address you
control. Confirm the selected **From** account, SMTP acceptance in Outbox, and a
single remote Sent copy after Sync. Restart and Sync again to check persistence.
For Gmail, also test Archive on a disposable message and confirm it remains in
All Mail. With a second account connected, verify From selection and independent
folder trees. Account credentials are entered locally, never through the chat.

## Service-independent mail tests

Run **`just test-mail`** for real SMTP/IMAP adapter tests against a disposable Rust
daemon, **`just test-maild`** to run it locally, or **`just test-mail-docker`** to
build and test its Docker image. Both TLS and STARTTLS are covered, with no email
provider accounts or secrets. CI builds the image and runs the same round trip.

See **[Local IMAP/SMTP testing](local-mail-testing.md)** for credentials,
ports, certificate trust, Docker commands, CI, and the supported protocol subset.

## Live Gmail CI

Standalone setup checklist: **[Gmail testing — setup for later](gmail-testing.md)**.

`.github/workflows/gmail-live.yml` runs the ignored integration test in
`crates/moonmail-network/tests/gmail_live.rs` on a macOS runner. Create **two
dedicated Gmail accounts** (not plus/dot aliases of one mailbox), enable 2-Step
Verification, and generate an app password for each. Add these GitHub repository
Actions secrets:

| Secret | Value |
| --- | --- |
| `GMAIL_TEST_A_EMAIL` | First test account's primary Gmail address |
| `GMAIL_TEST_A_APP_PASSWORD` | Its app password, without display spaces |
| `GMAIL_TEST_B_EMAIL` | Second test account's primary Gmail address |
| `GMAIL_TEST_B_APP_PASSWORD` | Its app password, without display spaces |

Once the workflow is on the default branch, choose **Actions → Gmail live round
trip → Run workflow**. Set repository Actions variable `GMAIL_LIVE_ENABLED=true`
to also enable the daily 07:23 UTC run. Runs are serialized, and only default-branch
code receives these secrets; pull requests do not trigger this workflow. Normal
`just ci` compiles the test but leaves live network testing ignored.

The test authenticates both IMAP/SMTP services, sends one uniquely tagged message
A → B with a UTF-8 body and synthetic attachment, syncs B, constructs Moonmail's
reply, sends B → A, and verifies both Inbox deliveries, reply ancestry, and exactly
one observed Sent copy per message. It reopens its local archives and reconnects
before checking again. Sending is limited to those two addresses, with no SMTP
retries; delivery polling has an eight-minute deadline. This exercises the real
network, MIME, local archive, outbox, reply, sync, and Sent-reconciliation paths.

Messages remain in the dedicated accounts for inspection, under subjects beginning
`[Moonmail CI]` followed by a UUID. The test does not delete mail. It syncs these
accounts into disposable temporary archives, so keep them small and synthetic.
No mailbox/credential artifacts are uploaded; logs contain phase labels and run IDs.
Gmail may delay or classify delivery as Spam, and Google account policy may prevent
app-password authentication. Failures are reported rather than treated as passes.
This does not cover OAuth consent, refresh, Keychain persistence, or the GUI.

To run locally with the same four secrets already exported in your shell:

```sh
MOONMAIL_LIVE_GMAIL=1 cargo test --locked -p moonmail-network --test gmail_live gmail_round_trip -- --ignored --exact --nocapture
```

## See a msgvault archive

Moonmail finds a [msgvault](https://github.com/wesm/msgvault) archive the way
msgvault does: `$MSGVAULT_HOME`, else `~/.msgvault`, with `[data] data_dir` in
its `config.toml`. When you open Moonmail, each msgvault source that holds
email becomes an account named **msgvault** followed by its address. Chats and
other non-email sources are left out.

A background worker copies each message's original MIME into Moonmail's
archive, oldest first, and shows its progress in the status bar. A large
archive takes a while the first time: about 30 messages a second, so roughly
45 minutes for 85,000 messages. Messages whose bytes Moonmail already holds,
such as the same account synced over IMAP or JMAP, share one copy on disk. The
import resumes where it stopped after a quit. Run `moonmail msgvault` to mirror
from the command line instead.

Under **Sources**, click an account's name to see only that account's mail, so a mailbox synced both directly and through msgvault does not show twice. The role groups under **Mailboxes** (Inbox, Archive, Sent, Junk, Trash) list each account separately, and **All Messages** shows every account together, duplicates included.

msgvault labels and IMAP folders become mailboxes, and a message without one
appears in **All Mail**. After each msgvault sync, Moonmail follows its folder
changes, read state and deletions. A deleted message leaves every folder; its
original stays in the archive. Moonmail opens msgvault's database read-only and
never writes to it, so moves and folder edits are refused for these accounts.
Marking a message read in Moonmail lasts until msgvault reports a new read
state. Set `MOONMAIL_MSGVAULT=0` to turn detection off.

## Share a live Maildir with Mutt and mbsync

1. Run your existing mbsync channel to populate its local Maildir. For example, if the channel is named `work`, run `mbsync work`. Use a local filesystem with stable inode numbers and atomic exclusive rename support.
2. In Moonmail, choose **Mail → Connect live Maildir…** and select the Maildir or the directory containing that account’s Maildirs. Its folders appear in the sidebar under **Maildir** followed by the selected path. Moonmail remembers the connection across launches.
3. Open the same mailbox in Mutt, for example `mutt -f ~/Mail/work/INBOX` if that directory contains `cur` and `new`. Deliver, move or expunge mail in Mutt or through mbsync. Moonmail scans on launch and two seconds after each background pass. Mutt may need a mailbox check or sync to publish its edits.
4. Mark a message read, unread or starred in Moonmail. The worker merges that change into the current filename, preserves other flag letters and the opaque comma suffix, and moves changed files from `new` into `cur`. The reader’s mark-as-read preference applies here too. Queued edits survive quitting the app. Run mbsync on your usual schedule to exchange those flags with the server.

The account reads published regular files in `new` and `cur`, skips `tmp` and symlinks, and supports nested folders and Maildir++ names such as `.Lists.Rust`. Filenames must be UTF-8 and use the standard unique name with an optional `:2,` suffix. The message size limit is 64 MiB. Remote login, sending, folder operations, and translating IMAP keywords belong to your other mail tools in this workflow.

Each delivery has its own identity, even when two deliveries have identical bytes. An unambiguous rename on the same filesystem preserves that identity. For a copy followed by an unlink within a scan interval, Moonmail can also recognize an unambiguous matching filename stem and identical bytes. A new inode and a new name together are treated as a new delivery. Expunged messages leave live views while their original bytes stay in the archive.

Maildir files are expected to be immutable after publication and clients must use unique delivery names. Moonmail compares file metadata before opening bodies and rechecks directory metadata before retiring missing entries. If a scan encounters an unreadable or partial mailbox, a changing directory, or conflicting filenames, the worker retries. A missing or replaced account root leaves the previous inventory available. Scans run while the app is open. The network Sync and Stop Sync controls apply to server connections.

## Local data and import

Default archive: `~/Library/Application Support/moonmail/`.
Demo archive: `~/Library/Application Support/moonmail-demo/`.
Set `MOONMAIL_DATA_DIR` to override the archive path. One process owns each archive;
close the GUI before using CLI commands against the same archive.

```sh
just import /path/to/Maildir
just import /path/to/mailbox.mbox
just import /path/to/eml-directory  # recursively read .eml; source files stay untouched
just run status
just reindex
MOONMAIL_DATA_DIR=/path/to/archive just run
```

Use **Mail → Import mail file…** or **Import mail folder…** to copy a snapshot
in the background. Choose a destination archive outside the source tree. Import
commits one message at a time, up to 64 MiB each. After an error, fix the source
copy and retry. Identical bytes share a local message across mailboxes.

- **Maildir.** Select a Maildir (its own messages land in `Imported`) or its
  enclosing directory. Paths such as `Work/Projects/2025/{cur,new}` retain all
  three folder names. Under a Maildir, `.Lists.Rust` maps to `Lists/Rust` and
  mbsync's `..Lists.Rust` maps to `INBOX/Lists/Rust`. The importer copies
  regular, non-hidden files in `cur` and `new` byte for byte and reads seen
  and flagged from the standard `:2,` filename suffix. It skips `tmp`, symlinks,
  and Maildir state files. Custom mbsync delimiters and flag mappings require
  conversion before import.
- **EML.** Individual and recursively discovered `.eml` files retain their
  original bytes. Loose EML files in an ordinary directory tree share the
  `Imported` mailbox.
- **mbox.** Select an extensionless Unix spool directly. Folder imports discover
  `.mbox`, `.mbx`, and `*.mbox/mbox`. Without a top-level `Content-Length`,
  an unquoted line beginning with `From ` separates records. The reader removes
  one level of mboxrd body quoting and one trailing empty framing line, if
  present. Older mboxo files can be ambiguous when a body already contained
  quoted From lines. With a decimal `Content-Length`, the reader copies exactly
  that many body bytes, including literal From lines and any stored quoting.
  Invalid lengths or framing stop the import. EML and mbox imports start as read.

Reimport picks up additions and refreshes Maildir flags. The archive keeps old
mailbox memberships after a source move or deletion, including folders that an
earlier import flattened.
For a consistent snapshot, import a stationary copy of the source.

- `archive.sqlite3`: authoritative metadata, bodies, flags, drafts, outbox, journal.
- `messages/<prefix>/<sha256>.eml`: immutable original bytes, verified on read.
- `attachments/<prefix>/<sha256>.bin`: immutable draft-file copies, verified on send.
- `search.sqlite3`: rebuildable SQLite FTS5 projection.
- `account.json`: ordered account registry and server configuration, no tokens/passwords.
  Legacy single-account files are read as-is and migrate atomically on the next write.
- Temporary reader files: sanitized local HTML, removed on normal shutdown.

Back up the **whole archive**, with the app closed. Local mail is not encrypted by
Moonmail; use the operating system's disk encryption as appropriate. Removing an
account connection retains local mail. Expunged/old UIDVALIDITY occurrences leave
live views; original bytes remain in the archive.

## Current limits

- Up to 32 accounts per archive; macOS Keychain persistence only. Google browser sign-in uses a packaged or imported client. Microsoft uses an imported desktop client. No POP3/EWS support.
- Sync fixes the UID upper bound at SELECT. Full inventories use UID SEARCH and
  fetch metadata for up to 700 existing UIDs per command. With CONDSTORE, matching
  UIDVALIDITY, UIDNEXT and HIGHESTMODSEQ skip the inventory. Changed mailboxes use
  CHANGEDSINCE for old flags and fetch metadata for arrivals. QRESYNC reports
  expunged UIDs directly. Plain CONDSTORE checks UID membership with SEARCH.
  SEARCH holds the UID list in memory, and the IMAP library buffers each command
  response. No million-message or live-account performance measurement is available.
- Sync stages metadata on disk and applies each completed inventory atomically.
  Body downloads batch up to 32 UIDs and 8 MiB of reported sizes. Larger messages
  up to 64 MiB download individually. A missing body or interrupted download keeps
  the mailbox eligible for a full retry. Completed downloads survive cancellation.
- **Settings → Synchronization** chooses manual, 1, 5, 15 (default), or 30-minute
  checks while the app is open. Transport failures reconnect with 30-second
  exponential backoff capped at 15 minutes. Authentication, TLS, storage and
  incomplete-sync errors pause the affected account until explicit Sync. Manual
  mode preserves existing manual preferences and disables IDLE and periodic reads.
  Edits still wake the worker to upload and reconcile. Stop Sync pauses those
  uploads too and stays paused across restarts until you choose Sync. Scheduled sync never submits queued messages.
- Remote message moves use native `MOVE` or the verified UIDPLUS fallback described
  above. Move preflight probes only the target UID, checks UIDVALIDITY,
  and rejects stale identities. The operation history shows at most 100 records
  with unresolved work first, plus a total unresolved count. Trash moves do not
  permanently expunge messages. Uncertain moves are not automatically repeated
  when synchronization reconnects.
- **Stop Sync** pauses automatic work and requests cancellation between blocking commands. An in-flight
  command or credential acquisition must return first; a remote mutation's
  outcome is recorded before stopping. Completed downloads and pending intent
  survive restart. Messages above 64 MiB are skipped with an explicit incomplete
  result, rather than claiming the account is up to date. Downloads commit
  individually and resume by durable remote identity. Metadata list pages contain
  at most 75 messages.
- New downloads retain both the Date header and server INTERNALDATE. Missing or
  malformed Date headers fall back to INTERNALDATE for sorting, never download
  time. The reader labels source/received time separately. Older archives retain
  their original sort dates with unknown date provenance rather than inventing it.
- Search includes Cc addresses and sender display names. Indexed term matches
  stream from one committed projection snapshot into disk-backed temporary sets.
  Boolean conditions and live metadata filters apply before sorting, threading
  and pagination; there is no global hit cap. Text sets are reused until the query
  or index changes, while mailbox/flag/date conditions remain live. Preparing a broad search still
  scans all its hits on the provider worker; background cancellation is not yet
  implemented. Existing archives replay the search journal once for these fields.
- Correspondence matches sender/recipient pairs in either direction, excludes
  Junk/Trash roles (with a name fallback only when no role is configured), and
  pages complete related history in 50-row pages with a separately pinned anchor.
  Subject/date sorting applies to the whole result before pagination. First/Next
  work for both Correspondence and Reply Thread; selecting a related message
  retains the original anchor. Restart at First to include newer arrivals.
  Reply threading uses account-scoped reference components. The main threaded
  listing uses the same graph, with a flat chronological expansion and
  First / Next 100 controls for complete matching history. Folder, flag and search
  scopes apply to every child page. Grouped
  queries compute ranks over matching metadata; they are not yet benchmarked on
  million-message archives.
- Plain-text composition, forwarding, configured identities/signatures, and
  IMAP-synchronized drafts; rich-text composition is not implemented. Raw MIME attachments
  are preserved. Self-exclusion recognizes the account and explicitly configured
  aliases. Restart draft pagination after edits to include newly promoted rows.
- Attachment inspection uses the `mailparse` adapter on demand (64 MiB raw input,
  at most 256 outer MIME nodes visited). Attached messages remain opaque `.eml`
  files. No automatic attachment opening or bulk export
  yet. Outgoing files use `application/octet-stream`; servers may impose lower
  message limits. Removing/discarding files leaves unreferenced local blobs;
  garbage collection is not implemented yet.
- HTML is sanitized with Ammonia and restrictive CSP. Scripts, forms, navigation,
  and remote images are blocked. **Links…** lists explicit HTTP(S) targets for
  opening in the system browser (100 links, first 512 KiB per alternative).
  **Embedded images** decodes local CID PNG/JPEG/GIF/WebP to static PNG; duplicate
  or missing CIDs are not guessed. Limits: 16 images, 2 MiB per input/output,
  8 MiB combined PNG output, 4096×4096 dimensions, 32 MiB decoder allocation.
  Plain text remains selectable in the same stable webview.
- No automatic SMTP retries. **Server settings → Sent messages** chooses provider-managed
  copies (default for Gmail/Fastmail) or IMAP APPEND (other presets). APPEND uses the
  configured, unambiguous Sent role; Sync first or assign the role in Move & folders.
  Copy intent is durable before SMTP. Pending copies resume on Sync; a lost APPEND
  reply or interrupted copy becomes **unknown** and never appends automatically again.
  Outbox retains accepted messages, exact submitted bytes, and copy status. After successful Sync, new Outbox entries
  show whether a unique matching Message-ID exists in the account's configured
  Sent folder. **Show Sent copy** opens that message. Missing/ambiguous matches
  never resubmit SMTP; observation does not prove delivery to recipients.
  Older entries without tracked IDs remain explicitly untracked.

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

IMAP is pinned to **`=3.0.0-alpha.15`**, as selected. Moonframe is pinned to upstream
revision **`f8a69f7cc5fe6708ab3f57022995946ba7b93e92`**, which includes merged mod-window
title-bar support. The temporary sibling-checkout overrides have been removed;
builds no longer depend on an unrecorded local Moonframe worktree.
SQLite is aligned with Moonframe at `rusqlite 0.40.2` to avoid duplicate native linkage.

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
`organize` and `operations` capture folder/role controls and synthetic queued,
failed, and uncertain move records.
Mail workflow captures synchronously prepare real provider state before rendering
the frozen snapshot, avoiding the native capture timer's startup race. `forward`,
`links`, and `images` cover forwarding and reader controls. Generated sanitized
reader HTML is saved alongside each image. Verify the actual CID rendering with:

```sh
just screenshot target/screenshots/reader-images.png images
swift scripts/check_reader.swift target/screenshots/reader-images.html_url.html target/screenshots/reader-webkit.png
```

The WebKit check loads the generated synthetic page, verifies its decoded local
image and absence of active elements/navigation, and exports the WebKit surface.
`compose` mounts the actual composer view at its native window's default size,
with populated To/Cc/Bcc fields, quoted text, and attached files;
`drafts` captures multiple independently saved messages. `attachments` imports a
synthetic nested-MIME fixture and captures its decoded file inventory.

The generic harness compares identical owned MIME extraction and compatible
search semantics, checking parity outside timed loops. Durable disk commits and
non-durable in-memory runs are separate. Corpus/setup work is outside timings.
Results in `target/criterion/` are synthetic, not real-account or million-message
performance measurements. Reopen tests are not power-loss durability tests.
