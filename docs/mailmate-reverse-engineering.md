# MailMate reverse-engineering study and Moonmail roadmap

- **Research date:** 2026-11-03
- **MailMate:** 2.0, build 6272 (bundled release notes identify it as a beta)
- **Moonmail baseline:** `93da79348d9b81031af23fd0cb06b8d8d358b63a`
- **Status:** local research document; intentionally uncommitted.

### Implementation follow-up — first correctness batch

The analysis below describes the baseline commit above. Subsequent local changes
address **C3** (Cc/sender-name projection and complete scoped search) and the
anchor-inclusion portion of **C4**:

- Search now streams a consistent projection snapshot into a disk-backed,
  connection-local match set. Live/folder/flag filtering and thread grouping run
  before page limits. There is no global 1,000-hit cutoff.
- The match set is reused until the query or index changes; its replacement rolls
  back on failure. Broad searches still enumerate all hits on the provider worker;
  asynchronous preparation/cancellation remains future work.
- Archive schema version 3 schedules a one-time durable search-journal replay to
  add stored Cc addresses and the primary sender's display name. This does not
  reconstruct every recipient display name from raw MIME.
- Correspondence reserves space for the anchor within its result limit, retaining
  old, retired, and recipient-less selections. History pagination and role-based
  junk/trash exclusion remain pending.
- New regression coverage exercises more than 1,000 matching IDs, scope/grouping
  before pagination, snapshot consistency, interrupted replacement, migration
  replay after restart, active-search index updates, and anchor retention.

No changes from this follow-up are committed. The next batch is recorded below.

Validation: `just format`, `just ci` (27 tests plus workspace Clippy), and
`just bench-check` passed. The native GPUI layout capture at
`target/screenshots/search-correctness.png` was inspected; the capture excludes
the embedded WebKit reader surface. CI initially encountered a type-inference
error in the locally overridden Moonframe checkout; a single explicit
`Context<Self>` annotation in its `crates/moonframe/src/mod_window.rs` unblocked
the build while preserving the surrounding in-progress changes.

### Implementation follow-up — second correctness batch

Local changes now address **C1**, **C2**, and **C5**:

- **Folder disappearance:** a complete successful LIST atomically reconciles one
  account's namespace, hides missing mailboxes, and removes their occurrences and
  queued remote flags. It retains messages/blobs and other accounts, including the
  separate local-import account. Empty LIST is authoritative; failed or duplicate
  lists roll back. Reappearing folders reuse their mailbox identity and reconcile
  server UIDs again. All listed hierarchy metadata is published before downloads.
- **Reply References:** schema version 4 retains original References/In-Reply-To
  at ingest and persists draft ancestry. Legacy messages lazily recover headers
  from immutable MIME, caching successful empty results too; failures remain
  retryable. Existing draft content and known parent IDs survive migration without
  invented ancestry. The reply action prefers References, then In-Reply-To.
  Submission appends the immediate parent once, deduplicates, and bounds output
  to 100 IDs while retaining the root/newest ancestors. Malformed ancestors are
  omitted; malformed direct parents fail explicitly. This repairs generated
  chains, not the separate root-key-versus-thread-graph limitation.
- **Credential lock scope:** `CredentialStore` owns the exclusion-guard contract;
  `Auth` holds it throughout reads, refreshes, writes, authorization, and revocation.
  Keychain lock filenames hash service plus exact account identity under
  `~/Library/Application Support/moonmail/credential-locks/`, independent of the
  archive path. Contention returns before accessing credentials or network.
  Stable lock files are never unlinked. Synthetic tests cover independent store
  instances, distinct identities, cross-process contention, successful release,
  error-path release, and read/write/delete guard lifetimes.

Validation passed: `just format`, targeted `moonmail-network`/`moonmail-store`
tests, `just ci` (35 tests plus workspace Clippy), and `just bench-check` (all
workloads exercised without performance claims). The provider integration test
checks reply-action ancestry and In-Reply-To fallback through saved drafts into
generated MIME. The native compose capture at
`target/screenshots/reply-correctness.png` was inspected: recipient, subject,
saved body, hierarchy, and bottom actions render without clipping. It is a GPUI
layout capture and excludes embedded native WebKit. No authenticated account test
has been performed, and this batch remains uncommitted.

### Implementation follow-up — mail-management slice

Schema version 5 and accompanying app/worker changes add:

- SPECIAL-USE discovery via complete `LIST ... RETURN (SPECIAL-USE)` when
  advertised, ordinary LIST otherwise, and intrinsic INBOX recognition. Malformed
  response tails fail the whole list. Display names do not infer remote roles.
- Persistent per-account Archive/Trash overrides, role-based sidebar shortcuts,
  and explicit handling of ambiguous/missing destinations. An unavailable
  overridden folder does not silently fall back to another server-role folder.
- Transactional batches of up to 100 explicit message moves with durable
  source-folder/UIDVALIDITY/UID preconditions. Local UID-less moves preserve the
  message ID in one transaction. Remote source membership remains until an
  acknowledgment; immutable MIME is retained throughout.
- A connection-owning native `UID MOVE` path. Preflight selects the source and
  fetches only the target UID; existing durable flags run first. Stale epochs,
  vanished UIDs/folders, and unsupported MOVE become visible failed operations.
  A failed pre-dispatch SELECT leaves intent queued for the next manual Sync.
- Crash/ambiguous-response handling: running operations recover as unknown,
  unknown operations are not automatically repeated, and normal subsequent sync
  recovers actual source/destination membership. Users may cancel queued work or
  dismiss an unknown record after inspecting both folders. Destination refresh
  remains separate from the acknowledged operation state.
- Archive/Trash toolbar actions, checkbox multiselection, Move & folders with role
  controls, and a Mail operations pane. A collapsed thread checkbox selects only
  its representative; changing navigation scope clears checked selections.
  UI history is capped at 100 records, unresolved first, with a separate total.

Native MOVE is the only remote mutation path in this slice. No copy/delete or
unscoped EXPUNGE fallback, permanent deletion, automatic scheduler/reconnect,
folder-management commands, or cross-account moves are implemented. Full folder
sync still materializes its UID snapshot. Live Fastmail interoperability remains
unverified; regression fixtures use synthetic adapters.

During this batch the temporary `moonframe-mod-titlebar` sibling checkout was
removed externally. Its title-bar work had merged upstream, so Moonmail now pins
`f8a69f7cc5fe6708ab3f57022995946ba7b93e92` and removes local path patches. This
resolves the previously documented unrecorded dependency-provenance issue.

Validation passed: `just format`, `just ci` (42 tests plus workspace Clippy),
`just bench-check`, and `git diff --check`. New fixtures cover localized roles,
malformed LIST tails, persistent overrides, atomic bulk rollback, local identity,
offline queue reopen, flag ordering, failed preflight, UIDVALIDITY changes,
missing destinations, unsupported MOVE, cancellation, interrupted dispatch, and
lost acknowledgments followed by restart/reconciliation. Provider integration
checks bulk actions, role overrides, scope-change selection clearing, and both
new UI panes. Native GPUI screenshots were inspected at:

- `target/screenshots/mail-management.png` — added toolbar controls and checkbox
  column retain the dense main-table/Correspondence layout.
- `target/screenshots/move-folders.png` — destination and role controls fit.
- `target/screenshots/mail-operations.png` — queued, failed, and unknown records
  display their relevant actions and total unresolved count.

Captures use isolated synthetic data and omit native WebKit surfaces. This batch
and the research document remain uncommitted.

### Implementation follow-up — composition and independent drafts

The next read/reply slice implements:

- Schema v6 retains From/Reply-To/To/Cc metadata and adds draft Cc/Bcc/account
  fields. Legacy recipient metadata is recovered lazily from immutable MIME;
  old draft contents survive migration. Both MIME adapters retain Reply-To.
- Display-name/group parsing, Unicode display names with ASCII mailboxes, and
  To → Cc → Bcc deduplication. Identity comparison preserves local-part case and
  folds domain case only. Bcc is kept locally and in the frozen delivery envelope,
  never recipient-visible MIME. Bcc-only messages use an undisclosed-recipient To.
- Reply-To preference, Reply All, known-account self-exclusion, replies to sent
  mail, preserved References, and quoted plain text capped at 64,000 source
  characters with an explicit truncation marker. Replies never reconstruct Bcc.
- Independent Compose/Reply drafts, metadata-only keyset pages, reopen/discard,
  millisecond edit ordering, and account-bound sending. Input/send events carry
  draft IDs so stale views cannot overwrite or send another draft. Queued draft
  IDs cannot be saved again; failed saves preserve the provider's saved version.

The UI includes To/Cc/Bcc, Reply All, and a paged draft browser. Drafts remain
local-only. Forwarding, attachments, signatures, aliases, rich composition, and
Sent reconciliation remain pending. Only the known account is excluded from
Reply All; unknown aliases are not guessed. Editing promotes a draft, so restart
pagination to include changes above an existing cursor. These are Moonmail
implementation choices, not additional claims about MailMate's decompiled code.

Validation passed: `just format`, `just ci` (48 tests and workspace Clippy),
`just bench-check`, and `git diff --check`. Fixtures cover Reply-To parity between
parsers, legacy recovery, display-name/group parsing, local-part case preservation,
Bcc-only envelopes, header controls, account mismatch, replies to sent mail, quote
truncation, 123-draft pagination/reopen, stale events, and injected SQLite save
failure. Native screenshots were inspected at
`target/screenshots/composition.png` and `target/screenshots/drafts.png`; populated
recipient fields, quoted text, draft count, and open/discard controls fit cleanly.
Moonframe lacks a `reply-all` icon, so the toolbar uses a labeled Reply All button
with its supported reply icon. Captures use synthetic data and omit native WebKit.
Live authenticated Fastmail testing remains pending. Changes and this report
remain uncommitted.

### Implementation follow-up — received and outgoing attachments

The attachment slice now provides:

- A separate backend-neutral `MimeAttachments` contract with typed errors,
  metadata-only inventory, message-scoped MIME-path IDs, and binary-preserving
  transfer decoding. The initial implementation uses `mailparse` on demand;
  ingestion and navigation do not extract attachment payloads.
- Received-file inspection and native Save As panels. Explicit attachments,
  named parts, inline binary images, and attached messages appear in the list.
  Attached messages are exported as opaque files, keeping their internal files
  out of the outer inventory. Suggested filenames are safe basenames. Exports
  publish complete decoded bytes without overwriting existing destinations.
- Native multi-file picking, attachment removal, and schema-v7 draft persistence.
  Up to 16 files / 25 MiB are copied into content-addressed `attachments/` blobs
  before one atomic draft edit. Source paths are not persisted. Changing or
  deleting the source file cannot mutate a saved draft. New blob namespaces are
  synced before committing metadata references.
- Generic outgoing preparation verifies every attachment blob and saved size
  before freezing multipart MIME. Missing/corrupt files fail preparation. Once
  queued, MIME is independent of draft blobs. Bcc stays envelope-only. Legacy
  preparation explicitly rejects drafts containing files instead of omitting them.
- File-picker replies retain draft IDs; export replies retain message and part
  IDs. Cancelled or stale callbacks cannot attach to another draft or export from
  a different selected message. A failed multi-file selection leaves the previous
  saved draft intact, though unreferenced copied blobs may remain.

Limits: 64 MiB raw messages, 256 outer MIME nodes visited, parser-specific recovery
for malformed MIME, and one attachment adapter so far. Outgoing files use
`application/octet-stream` and original file copies are not garbage-collected yet.
Automatic opening, inline CID rendering, bulk export, forwarding, and attachment
search remain pending. Server-side size limits and live Fastmail delivery still
require authenticated interoperability checks. This follow-up records Moonmail
implementation work; it adds no new decompilation claims about MailMate.

Validation passed: `just format`, `just ci` (53 tests and workspace Clippy), and
`just bench-check`. New synthetic cases cover nested MIME, root/unnamed/duplicate
files, RFC 2231 filenames, transfer bytes without charset conversion, attached
message opacity, malformed part IDs, explicit inventory limits, draft restart,
corruption/size mismatch, frozen outbox independence, atomic multi-file failure,
source edits/deletion, overwrite refusal, and stale/cancelled picker callbacks.
Headless Moonframe checks exercise native-picker request/answer routing with the
draft/message IDs intact. Actual picker dialogs were not automated.

Inspected native GPUI screenshots:

- `target/screenshots/compose-attachments.png` — two persisted files, sizes,
  removal controls, and the body/editor actions remain visible.
- `target/screenshots/received-attachments.png` — binary, text, inline image,
  and attached-message rows show decoded sizes and Save As actions.

Captures use isolated synthetic data and exclude native WebKit. All changes and
this report remain uncommitted.

### Implementation follow-up — recoverable Outbox

- Schema v8 retains an account-bound compose snapshot with each newly queued
  message. Queuing removes the editable draft in the same transaction. Navigation
  pages metadata only; exact lookup and keyset pages remove the old newest-100
  action cutoff.
- Queue only, cancel before submission, and Return to drafts are implemented.
  Restoration atomically cancels the original and creates a fresh draft with
  To/Cc/Bcc, ancestry, body, account, and attachment references intact. Frozen bytes
  and original-ID tombstones remain. Duplicate/stale restoration attempts cannot
  create multiple replacements. Pre-v8 entries without snapshots remain sendable
  or cancellable when eligible, but recovery data is never invented.
- The generic submission boundary classifies failures conservatively as unknown
  unless an adapter proves no submission began. Only a proven pre-submission
  failure permits an explicit retry. Retrying uses identical MIME and envelope.
  Cancellation/restoration compete atomically with the submission claim; losing
  the claim never calls the network adapter. Credential access follows account
  and state validation.
- SMTP acceptance still means SMTP acceptance, not independently reconciled Sent
  visibility. Unknown outcomes stay blocked from retry/restoration. Partial SMTP
  recipient failures remain conservative unknown outcomes.

### Implementation follow-up — forwarding, reader resources, and Sent observation

- Forward and Forward text only create independent new-conversation drafts with
  empty recipients and public original headers, never Bcc or reply ancestry.
  Attachment bytes are copied to immutable draft blobs; limits fail atomically
  with an explicit text-only alternative. Text truncation is disclosed.
- Reader Links lists normalized HTTP(S) targets, with credentials, controls,
  relative/local/custom schemes excluded. Actions bind to the selected message
  and an indexed provider-owned URL. No URL supplied by an event is opened.
- Embedded images resolve only local CID resources. Raster decoding/re-encoding
  strips metadata/animation; duplicates, oversized inputs, vector/active content,
  unknown CIDs and remote resources are rejected. CSP remains restrictive. The
  native Links pane is used because the pinned Moonframe browser lacks an
  appropriate pre-navigation interception interface for inline external links.
- Deterministic workflow captures now initialize the real provider synchronously
  and render its frozen synthetic snapshot. Forwarding, Links, and embedded-image
  controls were captured and inspected. A separate native WKWebView test loaded
  the generated HTML, verified a decoded 160×80 local PNG and no navigation/active
  elements, and captured `target/screenshots/reader-webkit.png`; GPUI alone cannot
  validate the WebKit surface. Reader CI passed 64 tests plus Clippy and the
  synthetic benchmark smoke workloads.
- Schema v9 tracks the frozen outgoing Message-ID independently of submission
  state. Successful synchronization observes matching remote-UID occurrences in
  the account's unambiguous Sent role folder. Unique, absent, ambiguous,
  unavailable and legacy-untracked cases remain distinct. Show Sent copy resolves
  the current unique match. Observation never promotes an unknown SMTP outcome
  to confirmed delivery or enables retry. Local imports and other accounts cannot
  satisfy the match. Generic-provider APPEND and live Fastmail validation remain
  outstanding.

### Implementation follow-up — explicit identities and paged relationships

- Schema v10 adds account-bound identities with stable IDs, display names,
  sending address, Reply-To and plain signatures. One default per account is
  changed transactionally, with rollback coverage. Drafts and Outbox recovery
  freeze identity values, preserving existing user edits after identity changes
  or deletion. From is emitted exactly once; the SMTP envelope uses the chosen
  address while credential lookup remains bound to the account.
- Reply selection prefers explicit owned To/Cc identities, or original From for
  own sent mail; Reply-All excludes the account and all configured aliases.
  Local-part spelling/case, dots and plus suffixes remain significant. Signature
  insertion occurs once on draft creation or by explicit command. Selecting a
  different identity never rewrites the body. Native identity-management and
  identity-compose captures use real provider state.
- The related-message contract now separates the pinned anchor from keyset-paged
  metadata. Correspondence and the existing root-key Reply Thread view have
  First/Next navigation and whole-result subject/date sorting. Query changes
  reset cursors; related-row selection retains the anchor. Junk/Trash exclusions
  use explicit/server roles before display-name fallback. Tests cover 237 matches,
  equal sort keys, both directions, non-live anchors, localized folders, role
  overrides, cross-account correspondence, and deletion between pages.
- This completes related-history paging, not RFC graph threading: missing-parent
  graph reconstruction, arbitrary nesting, and smart
  mailbox/query-language work remain on the roadmap. Likewise identities do not
  yet provide stable multi-account credential IDs or provider alias discovery.

### Implementation follow-up — sync dates and cancellation

- Schema v11 retains Date-header and source-received timestamps independently;
  the IMAP adapter requests/validates INTERNALDATE and uses it instead of download
  time when the Date header is absent or malformed. Legacy sort dates remain
  unchanged, with unknown independent provenance. Local imports without a known
  source timestamp remain explicitly unknown.
- A cancellable generic synchronization entry point checks between commands.
  Stop Sync never interrupts the atomic acknowledgment path after a remote
  mutation. Tests cancel before a move claim, after a successful remote MOVE,
  after a durable download, and after LIST but before namespace reconciliation.
  Completed work survives restart and does not repeat. Socket/credential calls
  already in flight still have to return; this is cooperative cancellation.
- Oversized-message skips now produce an explicit incomplete result instead of
  the previous unconditional Up to date status. Other durable downloads remain
  available, and skipped UIDs are not marked completed.
- Validation before this slice: 73 workspace tests and Clippy passed; synthetic
  benchmark smoke checks passed. The second Correspondence page was captured and
  inspected with 125 synthetic history messages and a separately pinned anchor.

### Implementation follow-up — expanded threads and staged synchronization

- Expanded main-list threads now keyset-page all matching members, with First /
  Next 100 controls immediately below the representative. Children retain folder,
  flag and search scope; representatives are not repeated among children.
  Duplicate page actions are rejected by cursor binding, hidden bulk selection
  clears on paging, and keyboard movement skips controls. A 237-member traversal
  test passes; the native expanded-thread and refreshed Sent-copy Outbox captures
  were inspected.
- The native IMAP inventory uses fixed UID ranges of at most 1,000 against a
  SELECT UIDNEXT boundary, avoiding sequence shifts during EXPUNGE. Metadata is
  staged in a disk-backed temporary SQLite table in bounded transactions. Only
  successful completion applies epoch, expunge and flag changes atomically.
  Pending local flag intent wins. Missing downloads use 100-row keyset pages,
  eliminating the synchronization path's full metadata vector and known-UID set.
  The compatibility snapshot API still materializes data for legacy callers.
- Tests cover 2,505 staged UIDs, duplicate batches, invalid/changing scope,
  injected merge failure/rollback, preservation of newer out-of-scope UIDs,
  failed new-epoch scans, restart, and a scripted rust-imap exchange with sparse
  ranges, EXPUNGE, final tagged failure, INTERNALDATE and consumer cancellation.
  Sparse UID spaces can require many empty FETCH commands; LIST and pending
  operation enumeration are still materialized. Full final reconciliation is
  atomic but may be a large SQLite write. No million-message claim is made.
- Automatic sync settings offer manual / 1 / 5 / 15 / 30-minute intervals while
  open. Monotonic scheduling skips catch-up bursts after sleep. One worker runs
  at a time. Only transport errors trigger 30-second exponential reconnect,
  capped at 15 minutes. Authentication, TLS validation, local-store errors and
  incomplete results pause until explicit Sync. Stop Sync also pauses scheduling.
  Scheduled work does not send Outbox messages or retry uncertain moves.
- Validation through staged inventory: 82 workspace tests and Clippy passed;
  synthetic benchmark smoke checks passed. Scheduling subsequently passed 86
  workspace tests plus Clippy (including capture-feature Clippy); its native
  settings capture was inspected.

### Implementation follow-up — typed search and saved queries

- A backend-neutral `MailPredicate` AST now represents Boolean groups/NOT,
  literal indexed field terms, account/mailbox equality, live flags, and absolute
  date boundaries. A bounded parser accepts implicit/explicit AND, OR, NOT,
  parentheses and quoted phrases, plus subject/body/from/to/cc/account/in/is/
  after/before fields. Date ranges use UTC midnight, inclusive after and exclusive
  before. Unknown fields and malformed expressions report errors.
- The search contract streams all term matches from one committed projection
  snapshot. SQLite stages `(term,id)` in a disk-backed table atomically; compiled
  Boolean predicates combine those sets with live metadata before grouping and
  pagination. No body scanning enters navigation queries. Failed preparation
  preserves the prior query and sets. Flag/folder changes take effect without
  rebuilding text matches. Input limits are 4 KiB / 64 conditions / 16 levels.
- Schema v12 stores up to 100 named source/AST pairs. Save with an existing name
  explicitly replaces it. Opening evaluates the saved AST against current mail,
  resets toolbar folder/flag scope, and preserves explicit in:/account: predicates.
  Saved queries are not yet nested smart-mailbox sources, rules or auto-partitions.
- Validation passed 91 workspace tests, capture-feature Clippy and synthetic
  benchmark smoke checks. Coverage includes mixed predicate paging, live flags,
  field isolation, concurrent index commits between term scans, restart,
  malformed syntax, rollback and provider preparation failures retaining the
  previous query. Populated saved-search and fielded-result native captures were
  inspected (`saved-searches.png`, `search-results.png`).

### Implementation follow-up — nested saved sources and mailbox choosers

- Schema v13 adds durable source dependencies between saved searches. Effective
  membership is the local predicate AND the union of source predicates; no sources
  means all live mail. Source graph reads share one metadata snapshot and their
  expanded indexed terms retain the existing single-index-snapshot semantics.
- Save validates all descendants before committing: self/transitive cycles,
  missing sources, excessive nesting and expanded condition budgets reject the
  complete edit. Shared ancestors are permitted within the same bounds. Parents
  cannot be deleted while referenced. Existing schema-v12 queries migrate with
  empty sources and unchanged ASTs. Active saved views re-resolve their sources
  on refresh, so parent edits invalidate matches even when child text is unchanged.
- The native Saved searches pane offers source selection, source labels and Edit.
  Opening a saved query shows its name in the title bar; direct toolbar submission
  leaves the source context. The nested-source capture was inspected. Three new
  store tests and extended provider coverage passed: 94 workspace tests plus
  Clippy and benchmark smoke checks. Automatic partitions, entry rules and IMAP
  keyword labels remain separate unfinished work.
- Keyboard mailbox/move choosers add path/account filtering, up/down/Enter/Escape,
  bounded displayed matches and same-account destinations. Move events bind the
  selected IDs and source mailbox; backend preconditions remain authoritative.
  Their native captures were inspected; selection highlights and independent
  result scrolling keep the filter visible. Tests reject changed selection,
  changed scope, cross-account destinations and repeated source moves.
- Main list pages now expose first/last keyset bounds, retaining group activity
  keys even on terminal pages. Previous uses a reversed bounded metadata query,
  and message up/down crosses main page boundaries. Coverage traverses forward
  and backward with date/sender/subject sorts in both directions, flat/grouped
  rows, ties, late thread activity and combined folder/flag/search scope.
  Virtual-list automatic scroll-to-selection is still an upstream UI limitation;
  keyboard selection and reader navigation work, but the selected row can require
  scrolling. Expanded thread subpages keep their explicit First/Next controls.

### Implementation follow-up — original-message export

- The reader's Save .eml action and `moonmail export MESSAGE_ID PATH` export exact
  original, hash-verified bytes without MIME reserialization or flag mutation.
  Complete bytes are synced to a private temporary file before publishing under
  a new name. Existing files and destination symlinks are not overwritten.
  Attachment export uses the same publication helper. Stale native save-panel
  callbacks are bound to the selected message.
- Tests verify folded/encoded headers and binary multipart content byte-for-byte,
  native-action and CLI paths, import roundtrip, existing files/symlinks, stale
  panels and corruption rejection before destination creation. The reader export
  controls were inspected in a native GPUI capture. Bulk mailbox/mbox export and
  archive backup/restore are still absent.
- Latest validation: 97 workspace tests and Clippy passed; capture-feature Clippy,
  benchmark smoke checks for the changed pagination contract, and diff whitespace
  checks passed. Captures remain synthetic/offline; native WebKit surfaces are
  excluded by the GPUI capture harness. Live authenticated interoperability is
  still unverified. All changes and this research document remain uncommitted.

### Implementation follow-up — graph threading, verified copy fallback, and thread commands

- Schema v14 adds account-scoped message/RFC nodes, provenance edges, and connected
  components. Missing ancestors and late parents join conversations; conflicting
  duplicate IDs lose own-ID links while identical-byte copies may share them.
  Explicit references remain, and cyclic input terminates. Group/expanded/related
  queries all use this projection. Header recovery updates edges atomically.
- Migration commits 100 messages per transaction and resumes after a failed page,
  without opening MIME bodies. Opening currently waits for completion. Graph links
  retain the oldest plus nearest 255 references when over limit; original headers
  remain complete. Reader diagnostics expose omitted links and conflicting IDs.
  Native captures `thread-graph.png` and `thread-warning.png` were inspected.
- MOVE now falls back to UIDPLUS COPY only when available. A usable single-UID
  COPYUID receipt is durable before source deletion. Destination epoch and exact
  SHA-256 bytes are verified with a bounded body fetch, then source epoch is
  rechecked before UID STORE/UID EXPUNGE. Ordinary EXPUNGE/CLOSE are never sent.
  Missing mappings, byte/epoch mismatch, lost replies, and cancellation after copy
  retain uncertain operations without automatic retries. Receipt-write failure
  prevents source removal; restart recovers the operation as unknown.
- Unread navigation filters after grouping, retaining stable representatives and
  sort keys. Collapsed unread indicators include matching unread children. It
  preserves folder/search/flag scope and reveals hidden unread
  children through bounded expansion pages. Next/previous thread commands skip
  expanded children. Whole-thread read/unread stages all member IDs on disk and
  commits flags and remote intents atomically; pending moves reject the entire edit.
- Synthetic tests cover graph merges/splits, account isolation, identical copies,
  cycles, long chains, ingest/header rollback, migration restart, targeted protocol
  commands, copy/removal ambiguity, cancellation, failed receipt persistence,
  237-member flag updates, and unread children beyond the initial expansion page.
  Full parent/child presentation, configurable bindings, folder CRUD, standalone
  copy, rules/keywords/partitions, broader accounts, crypto, and backup remain gaps.
  Live authenticated interoperability and million-message performance are unverified.
- Validation: 107 workspace tests, workspace and capture-feature Clippy, headless
  UI check, formatting, benchmark smoke checks, and diff whitespace checks passed.
  `unread-navigation.png` was also inspected: an unread member beyond the first
  100 replies is selected visibly beneath its 237-member thread header. Captures
  use isolated synthetic mail and exclude WebKit surfaces. All implementation
  changes and this research document remain uncommitted and unpushed.

### Implementation follow-up — durable folder editing and standalone copy

- Schema v15 journals CREATE/RENAME/DELETE with frozen account, names, hierarchy
  membership and UIDVALIDITY. One unresolved folder edit per account excludes
  concurrent message transfers. Rename supports at most 1,000 members, revalidates
  complete namespace membership and selectable-folder epochs before dispatch, and
  preserves local mailbox IDs/roles/occurrences after acknowledgment.
- Leaf deletion requires exact-name UI confirmation and retains downloaded original
  bytes. INBOX is protected. Native rename/delete first EXAMINE INBOX to deselect
  without expunging. A lost reply or failed local outcome commit remains uncertain;
  neither restart nor later sync automatically repeats a namespace mutation.
- Folder history reads bounded summaries without loading frozen descendant
  inventories. The manager binds callbacks to account, name and epoch.
- Schema v16 distinguishes standalone copy from move while preserving old journal
  rows as moves. Copies always retain the source. Native standalone COPY needs no
  UIDPLUS; tagged success without a usable COPYUID is complete because no removal
  follows. Normal sync discovers destination membership. Ambiguous replies and
  interrupted durable completion never trigger automatic duplicate copies.
- Local copies are atomic, retain message identity, and preserve existing
  destination flags. The copy chooser shares selection/scope/account validation
  with move and adds the default ⌘⌥M shortcut.

Coverage includes hierarchy/role preservation, leaf deletion, stale namespace and
epochs, cancellation on both sides of dispatch, lost replies, injected SQLite
completion failures, restart, schema upgrades, and native command transcripts.
Live authenticated interoperability, cross-account transfer, subscription controls,
and permanent individual-message deletion remain unfinished.

### Implementation follow-up — configurable columns and selection export

- Main-list From/Subject/Date/To columns have durable order, visibility and bounded
  widths in `preferences.json`. Subject remains visible/flexible; To is display-only.
  Controls use an exact base-layout token, so stale edits fail rather than toggling
  a newer layout. Invalid or failed saves retain the published preferences. Other
  settings preserve column choices, and legacy preferences receive defaults.
- Thread disclosure stays available on Subject when From is hidden. List identity
  includes the layout to invalidate cached virtual rows. Correspondence retains its
  compact Subject/Date view. Tests exercise populated layouts and settings views.
- UI/CLI export up to 100 explicit IDs as hash-verified original `.eml` files in a
  newly allocated private directory. Blob references freeze before writing; only
  one body is held at a time. A versioned manifest containing IDs, names, hashes and
  sizes is published last. Ordinary failures remove only the new staging directory;
  process interruption may leave a directory without its completion manifest.
- Export callbacks validate selected IDs and source scope. Repeated exports use
  independent directories. Import-roundtrip fixtures preserve exact MIME bytes,
  including From-like body lines, and corruption prevents a completed export.

These exports recover message contents, not full archive state. Recursive graph
presentation, per-mailbox column profiles, arbitrary columns, configurable key
bindings, keywords/rules/partitions, and verified backup/restore remain outstanding.

The same batch adds explicit selection read/unread and flag/unflag commands. Up to
100 IDs and all their live occurrences update in one transaction with durable
remote intents. Pending transfers, missing occurrences, or injected journal-write
failure roll back the entire selection. Repeated IDs are deduplicated. Provider
tests reject stale selections and keep an explicitly unread displayed message
unread. Folder-operation counts now participate in the sidebar's unresolved total.

Native GPUI captures inspected: `folder-manager.png`, `copy-chooser.png`,
`columns.png`, `column-settings.png`, `export.png`, and `bulk-flags.png` under `target/screenshots/`.
The settings capture injects a real precomputed provider snapshot to avoid capturing
its initial unloaded state. These are synthetic layout checks; WebKit surfaces
are excluded. No authenticated account was modified.

Validation passed for the completed folder/copy/column/export/bulk-flag changes:
`just format`, `just ci` (**123 workspace tests** and Clippy), `just bench-check`,
capture-feature Clippy, `just check`, and `git diff --check`. Benchmark smoke checks
exercise workloads without supporting real-account performance claims. All code
changes and this research document remain uncommitted and unpushed.

### Implementation follow-up — IMAP keywords and account-local tag labels

Schema version 17 adds occurrence-scoped keyword membership, separate account-local
friendly labels, and durable add/remove intent. Wire names are validated ASCII
atoms, case-folded independently of Unicode display labels. Ingest and complete
disk-staged inventories publish keyword sets atomically; failed/partial scans do
not clear authoritative membership. Pending and blocked local intent wins over
observations until acknowledgment or explicit discard.

Each replacement edit receives a fresh AUTOINCREMENT ID, so late acknowledgments
cannot erase A/B/A edits. Native STORE validates UIDVALIDITY, UID, read/write access,
and explicit PERMANENTFLAGS support before adding/removing one atom. Lost replies
and local acknowledgment failures remain idempotently retryable. Unsupported/stale
operations block visibly instead of spinning. The pinned adapter conflates absent
and empty PERMANENTFLAGS; both are conservatively unsupported in this implementation.

Keyword intent runs before transfers. Remaining blocked intent fails the queued
transfer before any COPY/MOVE. Local moves carry membership; repeated local copies
preserve an existing destination's state. `keyword:` and `tag:` predicates work in
direct and nested saved searches using live metadata. Label renames never mutate
wire membership. **Tags & keywords** supports selection-bound bulk actions, label
editing, per-account paged operation history, explicit retry and discard. The sidebar
unresolved total includes keyword intent. Incoming observations/local additions are
bounded to 128 keywords per occurrence, atoms to 255 bytes, and displayed labels to
500 per account; pending overlays can temporarily exceed the observation limit.

Validation: `just format`, `just ci` (**129 workspace tests** plus Clippy),
`just bench-check`, and `just check` passed. Tests cover rollback, invalid atoms,
case normalization, account-local labels, live search, migration, partial inventory,
epoch retirement, stale UI events, lost responses, cancellation, restart, unsupported
flags, and keyword-before-COPY ordering. `target/screenshots/tags.png` was inspected
with populated labels plus blocked/queued operations. Native WebKit is excluded.
No authenticated IMAP interoperability was exercised. Changes remain uncommitted.

### Implementation follow-up — atomic physical-folder entry rules

Schema version 18 adds up to 100 versioned physical-folder keyword rules and paged
matching-run history. Metadata conditions reuse the direct-search predicate compiler;
text conditions are rejected explicitly rather than substituting different matching
semantics. Rules execute in creation order after the entering occurrence's observed
keywords are published. Later rules see earlier actions. The shared bulk-keyword
implementation updates all live occurrences and journals remote intent.

Entry, actions and history share the ingest/local-transfer transaction. There is no
separately advanced rule cursor to lose or replay: a storage failure rolls back the
entry itself, and its next ingestion retries the whole transaction. Duplicate
ingestion, reindex, repeated scans and copying into an existing membership do not
run entry rules. Real exit/reentry and UIDVALIDITY replacement are new entries.
Duplicate local imports now preserve existing keyword edits. Changing/enabling a
rule never backfills existing membership; manual application is explicit and binds
the selected messages and exact rule revision.

Rules can add/remove keywords only, so they cannot cause recursive folder-entry
cascades. Pending transfers and keyword limits produce durable blocked history;
storage failures propagate and roll back. Manual application reevaluates current
conditions and commits one selection transaction. Run history retains the original
rule name/revision after editing or deleting the definition. New remote-entry
keyword intent uploads during the following Sync. The native rules editor and
populated history in `target/screenshots/entry-rules.png` were inspected.

Remaining rule work includes indexed text conditions with explicit readiness,
smart-mailbox membership transitions, other actions, bounded move-rule cascades,
and inherited muting. The subsequent partition batch is recorded below.

### Implementation follow-up — automatic saved-search partitions

Schema version 19 persists a saved search's optional sender-address, To/Cc-address,
or keyword partition field, cascading configuration on search deletion. Partition
queries group the entire prepared resolved predicate before paging, using distinct
message IDs and live unread metadata. Multiple sender/recipient/keyword values can
place a message in several groups. Empty values are omitted. Binary value ordering
and exclusive 100-value keyset pages cover all groups without a global cutoff.

New `sender-address:` and `recipient-address:` predicates use parsed metadata,
preserving local-part case while folding ASCII domain case. They also work in
entry-rule conditions and nested saved searches. Bcc is never inferred. Opening
a partition intersects its exact value with the saved predicate, retaining that
intersection across refresh, navigation, reindex and parent-search edits. Parent
opening or partition-field changes clear the narrowing. UI actions bind the saved
predicate/field token and current page, rejecting stale source and page actions.

Groups currently live in the **Saved searches → Partitions** panel; this does not
create recursive sidebar submailboxes or virtual-membership entry events. Synthetic
coverage includes 205 sender groups, multi-recipient membership, duplicate physical
occurrences, live keyword removal, field persistence/deletion/migration, and exact
filter retention after parent edits.

Validation completed **2026-11-04**: `just format`, `just ci` (**138 workspace
tests**, workspace Clippy), `just bench-check`, `just check`, capture-feature Clippy,
and `git diff --check` passed. Native GPUI captures `tags.png`, `entry-rules.png`,
and `partitions.png` under `target/screenshots/` were inspected with synthetic
populated state. These captures exclude WebKit. No authenticated account test or
personal archive mutation was performed. Code and this report remain uncommitted
and unpushed; MailMate parity is still incomplete.

#### Next implementation boundary: broader rules and graph presentation

Smart-mailbox rules need durable membership transitions and explicit index-readiness
and edit baselines. Mutually moving rules require bounded cascade handling. Recursive
graph presentation must distinguish missing parents, conflicting IDs, cycles and
multiple occurrences without hiding valid members beyond a page. Scheduled sending
needs separate durable authorization and an atomic due-time claim; ordinary queued
Outbox messages remain manual. Configurable bindings, broader providers, crypto,
and full archive backup/restore remain additional unfinished phases.

### Implementation follow-up — independent composer and account-aware From menu

The composer now uses its own movable/resizable native window while mailbox
navigation remains available. Drafts use dense message-style rows and the composer
uses a joined rounded blue split Send control. One reusable compose window is
currently supported; opening another draft replaces its contents after saved edits.

The chosen **01 — Compact native** From menu is an anchored popover with full
sending addresses, checkmarks, arrow/Enter navigation, Escape dismissal, and
**Manage accounts…**. Selecting an account also selects its SMTP configuration and
credential lookup. Aliases remain owned by their account. Account and frozen alias
are saved together without altering recipients, body/signature edits, attachments,
or reply ancestry. Edited/deleted configured aliases never rewrite frozen drafts.
Changed menu inventories dismiss the popover rather than retargeting a queued key.

`account.json` now holds an ordered registry of up to 32 connected Fastmail accounts.
Legacy single-account files remain readable and migrate on the next atomic write.
The first connected account remains the new-message default. Settings supports
adding/removing accounts and identity management switches between accounts. Sync
visits accounts sequentially, stopping on the first failure with completed work
durable. Draft queueing and outbox submission/retry resolve their owning account
exactly; disconnected accounts never silently fall back to the default. Blank legacy
drafts require an explicit From selection. Disconnecting retains local archive data.

Synthetic coverage exercises migration/order/capacity, rejected registry updates,
cross-account/stale sender events, preserved compose content and files, reopen,
frozen MIME sender and Reply-To, disconnected-account rejection before credentials,
and keyboard dismissal/selection/Settings routing. Native capture fixture:
`just screenshot target/screenshots/from-menu-native.png from-menu`; the resulting
native GPUI image was inspected with three accounts, an alias, and attached files.
Authenticated multi-account OAuth/IMAP/SMTP remains unverified. Stable credential
IDs independent of email addresses, provider alias discovery, broader providers,
scheduled sending, and multiple simultaneous compose windows remain unfinished.

Validation completed **2026-11-04**: `just format`, `just ci` (**144 workspace
tests** and Clippy), `just bench-check`, `just check`, capture-feature Clippy,
Python capture-harness syntax checking, and `git diff --check` passed. The native
From-menu capture was inspected. Code and this report remain uncommitted/unpushed.

### Provider expansion — Gmail and other IMAP services (2026-11-04)

The account registry now supports Fastmail, Google, Microsoft, iCloud, Yahoo, AOL,
and custom IMAP/SMTP presets. Settings offers email/MX discovery, native secure
password entry, OAuth-client import, configurable per-service usernames/ports/TLS,
connection checks, and reconnect. Registration follows successful authentication
to both services. OAuth tokens remain bound to their original provider endpoints.

Google/Microsoft use user-owned desktop clients, PKCE, state-bound loopback
callbacks, and XOAUTH2. Google's omitted refresh token preserves the prior token;
rotated tokens are saved. Microsoft uses a registered `http://localhost/` desktop
redirect with a dynamic port. Common Outlook.com domains use the documented
`smtp-mail.outlook.com`; organizational accounts use `smtp.office365.com`.
Disconnect revokes Google/Fastmail tokens and removes Microsoft credentials locally.
Gmail's advertised `\All` becomes Archive independently of localized folder names.

Schema v20 adds durable Sent-copy intent before SMTP. Gmail/Fastmail default to
server-managed Sent; other presets default to IMAP APPEND, configurable in Settings.
Pending accepted copies resume during Sync using an unambiguous Sent role. APPEND
errors and interrupted attempts become unknown and never repeat automatically;
observing a unique remote Message-ID resolves an uncertain copy without resending.
Existing outbox entries migrate without inventing APPEND intent.

Synthetic coverage includes preset/legacy semantics, credential endpoint binding,
localized Gmail roles, SMTP XOAUTH2 success/error exchanges, IMAP challenge response,
required STARTTLS refusal and no-read-ahead, migration, exact frozen copy bytes,
account isolation, and restart/lost-acknowledgment behavior. README includes OAuth
registration and live setup steps. No authenticated provider session has been tested.

Remaining: bundled/verified OAuth applications for one-click sign-in, real-provider
interoperability, Gmail-native label operations and X-GM-MSGID deduplication, remote
draft sync, separate incoming/outgoing passwords, stable credential IDs independent
of email, richer rules/composition, crypto, and scheduled sending. This implements
multi-provider IMAP/SMTP paths, not full MailMate parity or Exchange/EWS support.

Validation completed **2026-11-04**: `just format ci bench-check check` passed
(**155 workspace tests**, Clippy, synthetic benchmark contract checks, and headless
UI validation). Capture-feature Clippy, Python harness syntax, and whitespace
checks passed. Native GPUI captures were inspected:
`target/screenshots/gmail-setup-native.png`, `imap-setup-native.png`, and
`accounts-providers-native.png`. These use isolated synthetic data, including a
three-provider account registry; they do not establish live interoperability.
Development and this report remain uncommitted/unpushed.

#### Opt-in live Gmail CI (2026-11-04)

An ignored `gmail_round_trip` integration test and GitHub Actions workflow now
exercise two dedicated consumer Gmail accounts with app-password secrets. It uses
the actual MIME/attachment preparation, durable outbox submission, IMAP sync,
downloaded-message reply construction, and Sent reconciliation. Two synthetic
messages cover A → B and B → A; fresh connections and reopened temporary archives
verify both deliveries and unique observed Sent copies. Logs omit provider error
payloads and mail contents; generated messages stay in the test accounts for
inspection. Sending is constrained to the configured pair and never retries SMTP.

Manual default-branch runs are available once published. Daily runs require
`GMAIL_LIVE_ENABLED=true`; no pull-request trigger receives secrets. README lists
the four required secrets. This workflow has not run against Gmail yet: accounts,
secrets, and publication are still required. It does not validate OAuth or the GUI.
Local validation passed: `just format ci` (**157 tests passed; one live test
ignored**), `actionlint`, YAML parsing, and `git diff --check`. An explicit
credential-free invocation confirmed the live test refuses to run without its
opt-in variable, before opening any network connection.

#### Service-independent protocol fixture (2026-11-04)

Gmail setup is now saved separately in [`gmail-testing.md`](gmail-testing.md).
`moonmail-test-maild` provides an ephemeral Rust IMAP/SMTP fixture built on
`imap-codec`, `smtp-proto`, `rustls`, and `rcgen`. Two isolated synthetic accounts
can send/reply locally over implicit TLS and STARTTLS. Per-start test CA trust is
explicit in the real adapters; normal certificate/hostname verification remains.

The native library starts on ephemeral ports and joins workers on drop. The CLI
publishes readiness and public CA files. A multi-stage Dockerfile builds the same
daemon; CI runs native network tests on macOS/Linux and builds/tests the image on
Linux without provider secrets. `just test-mail`, `just test-maild`, and
`just test-mail-docker` expose these paths locally.

Round trips use actual MIME/attachments, durable Outbox, downloaded-message reply
construction, Sent APPEND or server-managed Sent, and reopened archives/connections.
Boundary coverage includes TLS/auth rejection, account isolation, no external relay,
read-only selection, flags, UIDs, MOVE/targeted expunge, atomic quota rejection,
and shutdown. The Docker image's two-mode round trip has passed locally, as has
the Linux daemon/network suite (**65 tests passed; one live Gmail test ignored**).
Final workspace validation passed: `just format ci bench-check` (**165 tests
passed; one live Gmail test ignored**, workspace Clippy, formatting, and synthetic
benchmark smoke checks), workflow `actionlint`, Python syntax checks, and
`git diff --check`.

This is a bounded protocol subset, not full IMAP compliance or proof of provider
interoperability. It has fixed folders, synthetic INTERNALDATE, no persistence,
and no OAuth, SEARCH/IDLE, or folder-management implementation. See
[`local-mail-testing.md`](local-mail-testing.md) for the precise limits and usage.
Gmail credentials and live verification remain deferred. Development and this
report remain uncommitted/unpushed.

### Ghidra setup and verification

Homebrew Ghidra **12.1.4** and OpenJDK **21.0.12.1** were already installed. REA
needed configuration rather than a second installation:

- `GHIDRA_INSTALL_DIR=/opt/homebrew/opt/ghidra/libexec`
- `JAVA_HOME=/opt/homebrew/opt/openjdk@21/libexec/openjdk.jdk/Contents/Home`
- Configuration: `~/.config/opencode/opencode.jsonc`, backed up alongside it as
  `opencode.jsonc.before-ghidra.backup` before editing.

REA 6.4.0 reports the matching ARM64 native decompiler available. A synthetic
ARM64 Mach-O smoke check actually decompiled `_moonmail_smoke` at `0x100000328`,
recovering its conditional arithmetic correctly. Evidence:
`ev_61735aa11e29a40063c03aa06609c8516554c8a54a3e9c789638ccd9249da819`.
Its source, executable, and complete Evidence JSON are retained under
`~/.local/share/moonmail-research/mailmate-6272-2026-11-03/ghidra-smoke*`.
This is installation verification, not additional MailMate behavior evidence.

REA's Ghidra adapter rejects universal Mach-O imports, so a separate ARM64 slice
of MailMate was created in the research directory without changing the installed
app (slice SHA-256:
`2fd68e50e674a99be43f3ebc60e7e6bbfe95d810830bc3602bfa30643b5e49d4`).
Its first import/function request exceeded REA's default 330-second startup
deadline. The retry with a 1,200-second startup deadline succeeded. Ghidra
decompiled `MmMessagesWindowController::applyRules_` at `0x10013af2c` as a direct
call to `applyRulesToAllMessages_` with a false/zero argument, corroborating the
Hopper wrapper observation. Complete Evidence JSON:
`~/.local/share/moonmail-research/mailmate-6272-2026-11-03/ghidra-rule-dispatch-long.json`;
Evidence ID `ev_95b70a3622708721c33edd55923415615f84f8ce8431b99f3648345b7dcd5182`.
The original 17 MailMate dossiers below retain their original Hopper Evidence IDs.

## Executive assessment

Moonmail already has a useful local-first foundation: immutable MIME blobs, transactional SQLite metadata, a rebuildable search projection, durable local flags/outbox state, bounded navigation, and a real bidirectional Correspondence view. Its main limitation is now **email workflow completeness and correctness**, rather than the broad shape of its interface.

MailMate's distinctive design is a combination of:

1. **A parsed-message query system** shared by search, smart mailboxes, related-message views, columns, and contextual identity selection.
2. **Three different relationships:** reply-reference threads, address-based Correspondence, and identity-subtracted correspondents. These should stay distinct in Moonmail.
3. **An account-action model** for synchronization and local changes, with per-mailbox scheduling and explicit connection states.
4. **A MIME-part-aware reader and composer**, including attachments, CID resources, alternative bodies, identities, and signatures.
5. **Deep keyboard and declarative customization**, built on real mail operations rather than UI-only commands.

The recommended sequence is:

> Fix known correctness gaps → complete reliable Fastmail sync and mail operations → make reading/replying/attachments usable → finish threading and scoped search → add smart mailboxes/rules → expand customization and providers.

Keep Moonmail's SQLite/blob/journal architecture. MailMate's custom database and staged-file persistence are informative evidence, not a reason to replace working storage. Keep the pinned `imap =3.0.0-alpha.15` adapter on its connection-owning worker.

**Immediate fixes:** reconcile vanished folders; preserve complete reply References; index Cc and eliminate search's pre-scope 1,000-result cutoff; guarantee Correspondence anchor inclusion; align credential-refresh locking with credential identity across archives. These are source-derived risks, not failures reproduced against a live account in this study.

**Daily-driver blockers:** move/archive/trash, background synchronization, complete reply/forward and attachment workflows, usable links/images, multiple drafts, recoverable outbox states, and Sent reconciliation. More visual polish can accompany these features, but cannot substitute for them.

## Scope, artifact identity, and evidence method

### Target and tools

| Item | Observed value |
| --- | --- |
| Application | `/Applications/MailMate.app` |
| Executable | `/Applications/MailMate.app/Contents/MacOS/MailMate` |
| Bundle identifier | `com.freron.MailMate` |
| Version/build | `2.0` / `6272` |
| Bundled release heading | Revision 6272, BETA, July 2, 2025 |
| Executable SHA-256 reported by REA | `32be332e1067cc069bfcad3f6cb23b61bf2b2e981245e673da24339b7b3c6854` |
| Container / analyzed architecture | Universal Mach-O, arm64 + x86_64 / arm64 |
| Analysis | REA 6.4.0 with Hopper Disassembler demo 6.6.1 |
| Overview | 36,128 analyzed procedures; 26,421 provider-defined strings; five segments |
| Focused decompilation | 17 function dossiers across Correspondence, threading, persistence, sync, SMTP, identities, rendering, and a rule-command wrapper |

Hopper was installed with the user's approval from the official distribution. Its code signature was verified; installation is at `/Applications/Hopper Disassembler.app`. The analysis document was `MailMate.hop`.

### How to read the claims

- **[D] Decompiled:** recovered code and calls support the stated behavior; this is static evidence, not original source or a runtime trace.
- **[R] Resource:** a shipped configuration or readable script establishes a definition. It does not establish which options the user has enabled.
- **[H] Help:** bundled help/release notes describe intended behavior, possibly with version drift.
- **[S] Source:** current Moonmail source establishes an implementation or gap; live interoperability is not implied.
- **Recommendation:** a proposed Moonmail design or acceptance condition, not a recovered MailMate implementation.

Evidence references `D01`–`D17`, resource groups `R01`–`R10`, and source groups `S01`–`S10` are defined in the final section. Labels are local report identifiers; the ledger includes full REA Evidence IDs.

The investigation used shipped application files and Moonmail source. It did not inspect personal messages or modify mail archives. No authenticated MailMate/Fastmail runtime experiment was performed. Earlier Moonmail UI screenshots and test runs are separate work, not evidence of MailMate parity here.

Hopper cannot expose complete function-body ranges through the public API used by REA. Indirect calls and Objective-C dispatch are incomplete, and C++ type/argument recovery is imperfect. Some large functions produce heavily duplicated pseudocode: conclusions below use identifiable calls, literals, and narrow control-flow observations rather than pretending that every recovered branch is trustworthy. An absent symbol-name match is not evidence of an absent feature.

## MailMate architecture and decompiled workflows

### 1. Correspondence is a query, not a thread [D, R]

`-[MmMessagesWindowController showCorrespondence:]` at `0x100137510` obtains the selected body's `from.address` and `#recipient.split.address`, escapes single quotes, builds bidirectional sender/recipient alternatives, sets search type `correspondence`, and invokes `performSearch:`. This handler expects one selected body part. [D01]

The Correspondence **layout** separately defines the relation as the union of the current selection and related messages in All Messages. In conceptual notation:

```text
selected
  UNION messages from All Messages where
    (sender matches a selected sender AND recipient matches a selected recipient)
    OR
    (sender matches a selected recipient AND recipient matches a selected sender)
```

Important shipped semantics [R01]:

- `#recipient` includes **To, Cc, and Bcc**, where those headers exist locally.
- Subject and References do not participate.
- All Messages excludes Junk/Deleted, but the explicit selection union retains selected Junk/Deleted messages and messages without recipients.
- For multiple selections, the layout combines any selected sender with any selected recipient; its comment explicitly warns that this is not independent per-message pairing.
- The toolbar action and layout are separate implementations; single-selection behavior in one does not constrain the other.

Moonmail has the correct basic relation for From/To/Cc. Finish its anchor/pagination/role semantics before adding identity normalization or multiselection. Do not silently substitute “all messages involving the same person”: that broadens the pair relation. A correspondents column should separately exclude known user identities.

### 2. Threading is a maintained graph with cycle handling [D]

Recovered functions show more than grouping by a string:

- `reference_threading_t::look_for_parents`, `0x100365ba0`, looks up a reference key in a hash map whose values are vectors of node pointers, skips the same internal message ID, and adds parent/child edges. It also records the referencing node in another keyed collection. [D02]
- `check_for_cycles`, `0x100365cfc`, uses two integer sets, traverses children, reports a message-reference cycle, and removes child/parent relationships. Nodes without remaining parents can be reinserted into a root-like collection. [D03]
- `add_msg_ids`, `0x1003661c4`, processes new nodes, updates thread IDs, attaches children, and invokes cycle checks. Its callback also calls parent lookup and links both edge directions. [D04, D10]

The second lookup collection is consistent with resolving relationships when messages arrive in a different order; exact late-parent policy is not fully established. Multiple node pointers per key support multiple candidates, but do not establish MailMate's complete duplicate-Message-ID policy. Likewise, the recovered types do not prove exact References versus In-Reply-To precedence.

**Moonmail implication:** persist the full reference sequence and separate graph construction from view grouping. Define stable behavior for missing parents, late arrivals, duplicate identifiers, malformed/self references, cycles, removals, and cross-folder copies. Subject grouping is neither the current MailMate evidence nor a substitute for a reference graph.

### 3. Persistence coordinates multiple file operations [D, H]

`database_t::save`, `0x1002b6de4`, serializes database metadata including `largestUsedBodyPartID` into `Database.noindex/Main.plist`, registering it with `atomic_saving_t::add_file`. This is one save component, not evidence that the whole database resides in this plist. [D05]

The background save implementation at `0x1004afed4` contains staged operation markers for files to write, move, and delete, as well as attributes and tags. It calls marker-writing helpers, rename/move helpers, safe-content writes, backup cleanup, and missing-file handling. Observable marker names include `.files_to_be_written.tmp`, `.files_to_be_moved.tmp`, `.files_to_be_deleted.tmp`, `.attributes.tmp`, and `.tags.tmp`. [D11]

This supports a **coordinated staged-file persistence mechanism** and corroborates help describing local message files plus indexes and recovery to a safe state. It does not establish exact fsync ordering, power-loss atomicity, crash replay order, or the complete startup recovery algorithm. The large decompilation is particularly unsuitable for asserting those guarantees without follow-up analysis and fault injection.

**Moonmail implication:** preserve immutable blobs, WAL/FULL metadata transactions, durable change records, and search commit-before-acknowledgment. Define and test recovery of each boundary. Do not copy the plist/file-marker design merely to resemble MailMate.

### 4. Synchronization is scheduled account work [D, H]

`imap_account_t::synchronize`, `0x1002eff68`, consults account state, queued/active work, connection capacity, mailbox flags, recent throttling, timers, and create/delete mailbox conditions. It constructs actions and submits them through `account_t::set_action`; synchronization itself is delegated through `setup_synchronize_action`. [D06]

`synchronize_account`, `0x1002f4a0c`, contains watch setup, online/reachability checks, active/queued work checks, and mailbox configuration including `highest_modseq`. A MODSEQ key alone does not establish which incremental protocol variants are enabled or correct. `get_synchronization_interval`, `0x1002efc18`, delegates to mailbox scheduling; exact timing policy was not recovered from its numeric branches. [D07, D09]

Bundled account help supplies the user-visible scheduling model [R05]:

- Schedule inheritance from parent/account; Inbox defaults to Connected.
- Connected uses IDLE; help recommends only two or three such mailboxes per account.
- Periodic and manual schedules; selecting a mailbox or making local changes can trigger sync even in manual mode.
- Explicit offline, system-unreachable/disconnected, application-detected unavailable, persistent failed, and throttled states are distinct.
- Server subscriptions and local subscriptions are separate.

**Moonmail implication:** introduce a bounded, cancellable account scheduler around the existing connection-owning IMAP worker. Give local operations, selected-mailbox refresh, periodic discovery, IDLE, and full reconciliation explicit priorities. Begin with a correct full-reconciliation baseline; incremental extensions need a trustworthy fallback.

### 5. SMTP is action-based, with a duplicate-send guard [D]

`smtp_account_t::send_message`, `0x10037b50c`, checks for duplicate attempts involving the same internal message, validates that the message has a root body part, reads delivery-status-notification settings, and schedules an account action. Its diagnostic explicitly calls sending the same message twice a fatal condition. [D12]

`smtp_account_t::synchronize_account` contains reachability/server logic, while `did_finish` updates a progress-related field and releases connection state. These functions do **not** establish durable exactly-once delivery, retry policy after a lost final SMTP response, or Sent-folder append behavior. [D08, D13]

**Moonmail implication:** retain the conservative unknown-outcome state. Distinguish proven pre-submission failure, explicit rejection, partial-recipient outcomes, accepted submission, and lost acknowledgment before adding retries. An in-memory duplicate guard is not a substitute for the durable outbox Moonmail already has.

### 6. Identity selection reuses the query system [D, H]

`identities_manager_t::identity_for_recipients`, `0x1002edde0`, escapes recipient strings and builds `$Sent.filter(to.address = '…')`, obtains/sorts a message set, reads From, tries `identity_for_from_string`, and has a source-default fallback. [D14]

This directly corroborates the documented recipient-history mechanism. The exact sort direction, candidate tie-breaking, and interaction with a user's explicit From override were not fully reconstructed. Help separately describes contextual initial identity, reply-derived identity, aliases/patterns, and recipient-sensitive signature selection. [R04]

**Moonmail implication:** identities should be concrete account-owned records with explicit user override, not heuristics embedded in the UI. Preserve provenance such as “chosen explicitly,” “reply recipient,” “recent Sent history,” or “account default,” so suggestions are explainable and stable.

### 7. Rendering understands MIME resources and blocking state [D, H]

`MmCIDSchemeHandler` at `0x1001e932c` resolves URL data against a body part/draft context, emits MIME-typed responses, rewrites CSS according to image-blocking state, and reports missing embedded resources. [D15]

`MmBlockHTTPImgSchemeHandler` at `0x1001eb924` consults resource/blocking metadata, handles cached responses, rewrites CSS, and creates an SVG blocked-image presentation. Unknown resource information has explicit failure/reporting paths. [D16]

The release notes describe per-MIME-part views, WKWebView HTML, attachment views, a recently viewed subview cache, custom-scheme embedded resources, tracker/HTTP blocking, and individually loadable resources. This newer description supersedes older help's WebView account. [R06]

**Moonmail implication:** evolve the reader into a MIME-part presentation model with local CID resolution, attachment inventory, safe link opening, and explicit external-resource requests. Blocking network resources should not require deleting every useful link or every embedded image. Cache behavior must distinguish “display cached bytes” from “perform a new network fetch.”

### 8. Rule dispatch was located; trigger semantics remain documentation-backed [D, H]

`-[MmMessagesWindowController applyRules:]`, `0x10013af2c`, delegates to `applyRulesToAllMessages:` with a false argument. This is only a UI-command wrapper. It does not recover the evaluator or prove entry-trigger semantics. [D17]

Help explicitly says mailbox rules run when messages are **added to the mailbox**, with manual application available. Treat that event model as a documented requirement to evaluate, rather than claiming it was decompiled. [R03]

## Feature inventory and Moonmail gap analysis

“Present” means implemented in the reviewed Moonmail source; “partial” means useful foundations with important limits. This is not a percentage-complete estimate.

| Capability | MailMate evidence | Moonmail implementation (updated through follow-ups) | Next useful increment |
| --- | --- | --- | --- |
| Local archive | Local message files, indexes, staged saves [D05/D11, R05] | **Present:** SHA-256 immutable MIME, transactional metadata, separate search journal [S01] | Integrity/repair, backup/restore verification, crash-boundary tests |
| Accounts/auth | Multiple IMAP accounts, shared SMTP, aliases, manual settings [R05] | **Partial:** 32-account registry; provider presets/custom TLS IMAP/SMTP; Google/Microsoft desktop OAuth import, Fastmail OAuth, app passwords; explicit account-aware identities | Live interoperability, bundled OAuth clients, stable account IDs, shared SMTP configuration |
| Sync | Per-mailbox schedules, IDLE, account actions and connection states [D06–D09, R05] | **Partial:** staged UID-range scans, UIDVALIDITY/expunge handling, INTERNALDATE, cancellation, account-wide intervals and reconnect | Per-mailbox policy, IDLE/incremental sync, live interoperability |
| Mail/folder operations | Universal destinations, move chooser; create/delete conditions in native sync [D06, R03/R05] | **Partial:** durable flags, bulk move/copy/archive/trash, folder CRUD, role overrides, uncertainty recovery and verified UIDPLUS move fallback | Subscription policy, cross-account transfers, live interoperability |
| Threading | Parent/child graph and cycle removal [D02–D04/D10] | **Partial:** account-scoped reference components, missing/late parents, cycle termination, conflicting-ID isolation, retained headers and paged flat expansion | Recursive parent/child presentation, migration progress, large-component performance measurements |
| Correspondence | Bidirectional From ↔ To/Cc/Bcc; explicit selection union [D01, R01] | **Partial:** From ↔ To/Cc; pinned anchor, keyset history, role-based junk/trash exclusions | Define broader account/multiselect/Bcc behavior |
| Search | Fields, Boolean expressions, parsed headers, quoted/unquoted text, filename/date scopes [R02] | **Partial:** field/Boolean/UTC-date parser, uncapped scoped paging, Cc/name projection, atomic query preparation | Arbitrary headers, attachment fields, quoted/unquoted scopes, relative dates |
| Smart mailboxes | Nested source/condition sets, cross-mailbox membership, automatic partitions [R03] | **Partial:** durable named predicates and nested source unions, cycle/complexity validation, live refresh, persisted sender/recipient/keyword partition fields and complete paged groups | Incremental membership and automatic sidebar submailbox hierarchy |
| Tags/rules/muting | IMAP keyword mapping, mailbox-entry rules, inherited mute behavior [R03] | **Partial:** occurrence-scoped keywords, account-local labels, durable bulk edits/recovery, live keyword/tag search, atomic physical-folder metadata/keyword rules | Indexed conditions, smart-mailbox entry transitions, other actions and bounded cascades; inherited muting |
| Composer | Plain-text-first, Markdown HTML alternative, original HTML reply preservation [R04] | **Partial:** independent paged drafts, To/Cc/Bcc, Reply-To/Reply-All, quoting, forwarding, attachments | Rich HTML/Markdown reply preservation, broader compose editing |
| Identity/signature | Context/history/pattern matching and signature preferences [D14, R04] | **Partial:** explicit account-bound identities/signatures and alias-aware replies; drafts freeze identity | History/pattern suggestions and multiple-account UI |
| Delivery | Scheduled send documented; native duplicate-send guard [D12, R04] | **Partial:** frozen MIME/envelope, durable claims, safe retry/edit/cancel, independent Sent observation | Recipient-level outcomes, Sent append, Send Later |
| Attachments | MIME-part views, Quick Look, attach/save, CID handling [D15, R06] | **Partial:** bounded part inventory/extraction, native save/add, immutable attachments and forwarding | Quick Look/open workflow, richer part presentation |
| HTML reader | WKWebView, blocked-resource model, cached/embedded resources [D15/D16, R06] | **Partial:** sanitized HTML, explicit HTTP(S) links, bounded validated CID rasters | Broader resource policy/feedback and MIME alternatives |
| Crypto | S/MIME via Apple; OpenPGP through external gpg [R06] | **Missing** | Verification/status first if needed; signing/decryption after MIME model |
| Navigation | Thread/message/unread commands, mailbox/move chooser, multistroke bindings [R07] | **Partial:** sorting, folder tree, bulk selection, choosers, bidirectional pages, unread/thread seek and atomic thread read/unread | Configurable multistroke bindings and automatic list scroll-to-selection |
| Appearance/layout | Declarative layouts/columns, fonts/CSS and overrides [R01/R07] | **Present foundation:** resizable panes, settings/fonts, durable main-list column order/visibility/widths [S07] | Per-mailbox profiles, arbitrary columns, persisted sort choices |
| Automation/export | Bundles/filters, selected AppleScript commands, emate, folder/mbox export [R08/R09] | **Partial:** CLI tools, exact single-message and bounded selection .eml exports with completion manifests | Whole-mailbox/mbox export, backup/restore, stable command/extension API |
| Scale | No comparative runtime measurements in this study | **Unproven at large-account scale:** bounded UID ranges and disk staging; metadata grouping/final reconciliation can still be large | Synthetic 100k/1M query and concurrent-work benchmarks; later disposable live accounts |

### Search and smart-mailbox semantics worth retaining

MailMate's query expressiveness is more significant than copying its exact syntax [R02/R03]:

- Toolbar syntax includes AND/OR, parentheses, `!`, field modifiers, and arbitrary parsed-header paths.
- `f`, `s`, `t`, `a`, `d`, `b`, `q`, `m`, `M`, `T`, `K`, and `A` cover From, Subject, recipients, addresses, date, unquoted/quoted body, common scopes, tags, keywords, and attachment filenames.
- Common search includes addresses, subject, unquoted text, and filenames; Common+ adds quoted text.
- Relative date forms are calendar-aware: documented `1y` means this year, not a rolling 365-day duration.
- Smart mailboxes combine source sets and predicates with nested Any/All; “is in” can compare against header values in another mailbox.
- Multivalued partition keys can place one message in several automatic submailboxes.
- The shipped Personal Inbox uses recipient membership in Sent From addresses; its thread-expanded variant uses thread-ID membership.

The readable Ruby `generate_filter_script` corroborates a grammar/conversion approach, but the shipped Search command invokes the binary `generate_filter`. Exact parser parity and malformed-input behavior remain unresolved. Moonmail should define its own versioned grammar and semantic tests rather than treating the readable script as the authoritative current parser.

### Composition and organization details that affect the model

- Preserve Bcc for local composition/envelope purposes without exposing it in recipient-visible delivered headers. Received messages often do not contain Bcc; neither client can infer missing hidden recipients.
- Separate transport account, sending identity, From, Reply-To, and envelope sender. Provider support can constrain permitted combinations.
- Plain text plus optional Markdown-generated HTML matches MailMate's documented approach; a WYSIWYG editor is not a prerequisite. Original-HTML preservation is a later, separate MIME feature.
- Friendly tag labels are local mappings; IMAP keywords are the synchronized values. Renaming a label is not automatically a remote message mutation.
- Mailbox-entry rules need transitions, not repeated evaluation of “currently matches.” Local moves, server arrivals, and smart-mailbox changes need explicit event definitions and replay behavior.
- MailMate documents muting that propagates read/mute state and can follow the parent's mailbox, except directly addressed identity subthreads. This depends on graph, identities, and durable moves; implement it after those primitives.

## Concrete Moonmail correctness findings

These findings come from source inspection. They should become regression fixtures before broader feature work.

### C1. Server-deleted folders are not reconciled

`crates/moonmail-network/src/sync.rs:29–95` iterates folders returned by LIST and reconciles each selected folder, but has no complete-list reconciliation step for previously known folders now absent. Existing occurrences can remain live indefinitely. This is distinct from expunges within a still-existing folder, which are handled.

**Fix:** commit folder-list generations only after a successful complete enumeration; mark absent remote folders inactive and reconcile their occurrences/pending operations while retaining archived bytes. A failed or partial enumeration must never be mistaken for an empty authoritative list. Distinguish deletion from subscription/visibility changes where the adapter permits them.

### C2. Generated reply References can split threads

`archive.rs:135–141` chooses the first References value as its thread key. `provider.rs:495–505` keeps only the selected parent's RFC ID for a reply, and `submission.rs:102–105` writes only that value into References. Replying to a non-root message can therefore create a new root-key group.

**Fix:** retain ordered References and In-Reply-To in stored detail/drafts; append the parent's ID to the appropriate existing chain, with bounded malformed-input handling. Follow with a graph migration/rebuild. Keep frozen outbox bytes immutable.

### C3. Search loses valid scoped results

`archive.rs:182–199` fills `SearchDocument.cc` with an empty string; From display names are also not projected. `provider.rs:99–111` requests 1,000 global ranked IDs, then `queries.rs:35–74` applies live/folder/filter scope. Retired messages and messages in other folders can consume all candidates before a valid scoped hit is considered.

**Fix:** index the intended address/name fields and rebuild the projection. Replace the fixed candidate list with a search contract that can exhaustively page a scoped result. A bounded iterative candidate scan is a possible intermediate implementation only if it continues past filtered candidates and exposes honest exhaustion/cancellation. Simply raising the cap is not a correctness fix.

### C4. Correspondence's anchor can fall outside LIMIT

`queries.rs:165–179` includes `m.id=?1` in the predicate but applies a final newest-first LIMIT to the entire set. More than 100 newer related messages can exclude the app's selected anchor despite the comment promising to retain it.

**Fix:** return the anchor separately from a paginated related set, or explicitly reserve/include it after bounding related IDs. Define whether the visible limit includes the anchor, deduplicate it, and preserve the original anchor when reading a related message. Replace the four exact junk/trash folder names with mailbox roles and an explicit exclusion policy.

### C5. Credential identity and refresh-lock scope differ

`credentials.rs:25–47` keys Keychain entries by service/email, but `auth.rs:86–97,234–276` uses an archive-local authentication lock. Two archives for the same email can share a rotating refresh token without sharing the lock.

**Fix:** use a stable credential identity and lock at that scope, or isolate both credentials and locks per independent account instance. Coordinate refresh-token persistence and interrupted-refresh recovery. Preserve intentional reconnect behavior until a proven recovery mechanism replaces it.

### Additional operational limits

- Sync builds complete UID sets and obtains two snapshots per selectable folder; oversized messages are reported as skipped, while fetch/parse failures abort the run. Persist quarantined/skipped outcomes and useful retry state instead of presenting a permanently partial archive as complete. [S03]
- Incoming fallback `received_at` currently uses download time, not IMAP INTERNALDATE. Keep header Date, server received/internal date, and local ingestion time distinct. [S01/S03]
- Separate folder occurrences currently originate separate message records even when MIME bytes deduplicate. Do not merge solely on Message-ID; define copy/logical-message identity and conflicting-header policy. [S01/S03]
- SQLite provider callbacks run on Moonframe's native-provider worker, not necessarily the renderer thread. The practical concern is serialized mail-action latency when indexing and navigation share that worker. [S07/S10]

## Recommended architecture and implementation sequence

### Keep the current boundaries

| Layer | Responsibility to retain or extend |
| --- | --- |
| `moonmail-core` | Concrete message/account/query/action contracts; no parser, database, networking, executor, or UI dependencies |
| `moonmail-mime` | Interchangeable extraction into a bounded, typed MIME/header model; preserve raw MIME as the authority |
| `moonmail-store` | Authoritative metadata, occurrences, identities, durable operations, outbox, migrations, transactionally emitted changes |
| `moonmail-search` | Rebuildable projection and scoped, paginated search semantics; not authoritative mailbox state |
| `moonmail-network` | Narrow incoming/submission/credential boundaries; capability-aware adapters and connection-owning workers |
| `moonmail-app` | Commands and bounded view snapshots; cancellation/progress; lazy bodies and MIME-part presentation |
| `moonmail-bench` | Shared synthetic corpora, semantic parity checks outside timed loops, separate durability/navigation/sync measurements |

Prefer concrete data and static dispatch at replaceable boundaries. Keep structured library errors. Do not introduce a universal storage trait or make the UI wait for a full archive scan.

### Foundational model additions

These are proposals, not schemas recovered from MailMate:

1. **Account and identity records:** stable account ID, provider/capabilities, credential reference, owned sending identities, default folders and synchronization policy. Email addresses must not be the sole mutable identity of every resource.
2. **Messages, parts, and occurrences:** immutable raw blob; extracted typed headers and MIME tree; ordered References; part IDs, content disposition/type/filename/CID; mailbox occurrence keyed by account/mailbox/UIDVALIDITY/UID; distinct dates and local extraction version.
3. **Mailbox metadata:** hierarchy, selectability, subscription, special-use roles, remote-list generation, sync checkpoint and current user-visible state.
4. **Durable operations:** stable operation ID, intent, remote preconditions, attempt/acknowledgment state, definitive versus ambiguous failure, dependency information, and conflict result. Every UI command and rule should use the same operation path.
5. **Typed query AST:** Boolean composition, account/mailbox/live scope, parsed header/address predicates, dates, text scopes, tags, attachment fields, and later mailbox-value membership. Use one semantic model for toolbar input, saved searches, and smart mailboxes.
6. **Thread projection:** graph nodes/edges and stable view IDs derived from retained headers. Missing ancestors may be represented explicitly; rebuildability should not alter immutable message data.

Scope and pagination must be part of query contracts, including deterministic tie-breaks and what happens when sync changes a view between pages. Saved queries with mailbox dependencies need cycle detection. Calendar-relative predicates need timezone and refresh-boundary semantics.

### Phase 0 — Close correctness gaps and establish evidence

**Deliver:** C1–C5 fixes; synthetic regression fixtures; a reproducible build/dependency record; a disposable Fastmail acceptance harness for OAuth/app-password, IMAP, and SMTP.

Add crash/interruption points to existing storage and protocol fakes. Record current supported behavior and incomplete/skipped sync states in the UI. Resolve the README/local Moonframe dependency-provenance discrepancy before claiming clean-machine reproducibility.

**Exit:** all five fixtures pass; a failed enumeration cannot retire folders; a reply-to-reply keeps its chain; scoped search finds hits beyond 1,000 global competitors; old anchors remain visible; shared credentials cannot refresh concurrently without coordination.

### Phase 1 — Reliable Fastmail and mail management

**Deliver:** special-use folder discovery/overrides, complete folder reconciliation, bounded UID processing, INTERNALDATE, scheduler/cancellation/progress, offline intent, reconnect/backoff, and durable copy/move/archive/trash/folder operations.

Implement remote actions with capability-aware fallbacks. For copy-then-delete fallback, verify the destination outcome before deleting the source. UIDVALIDITY changes must invalidate stale remote preconditions. Bulk selection should enqueue durable intent, not synchronously perform a network loop on the provider/UI path.

Start with bounded polling plus explicit refresh, then add IDLE and incremental sync with full-reconciliation fallback. Keep manual, user-offline, unreachable, authentication-failed, and throttled states distinct.

**Exit:** offline changes survive restart and converge after reconnect; partial operations are diagnosable; no local checkpoint gets ahead of durable work; opening a message stays responsive during sync.

### Phase 2 — Complete the everyday read/reply/send loop

**Deliver:** MIME-part inventory and attachment open/save/add; safe external links and CID images; Cc/Bcc, Reply-To/Reply-All, quoting, forwarding, multiple drafts; explicit identities/signatures; full References preservation; outbox retry/edit/cancel for safe states; Sent reconciliation.

Retain the selected MIME alternative per message where useful. Distinguish cached resources from new downloads. Compose recipient parsing should handle display names and deduplication without changing envelope/header semantics. Bcc must reach the envelope without disclosure in delivered headers.

Separate “SMTP accepted” from “copy visible in Sent.” Some providers save sent mail automatically; others require append. Track the Sent-copy operation independently, so an append failure does not resubmit SMTP. Partial-recipient acceptance needs recipient-level outcomes before a safe retry can be offered.

**Exit:** a user can receive, inspect attachments, follow a link, Reply-All from the right identity, attach a file, resume a draft after restart, send, and locate the result in Sent. A lost final SMTP acknowledgment never causes automatic duplicate sending.

### Phase 3 — MailMate-quality relationships and search

**Deliver:** graph-based threading, pagination of expanded threads and Correspondence, role-aware exclusions, scoped search pages, field/Boolean/date syntax, quoted/unquoted indexing where supported, and saved searches.

The C2 fix and header storage belong earlier; this phase completes graph semantics and presentation. Define cross-account and cross-folder visibility explicitly. Do not infer alias equivalence by stripping plus suffixes globally; use configured identity policy.

**Exit:** adversarial reference graphs terminate deterministically; late arrivals update relationships without losing selection; search has no arbitrary recall ceiling; related history remains navigable beyond 100 messages.

### Phase 4 — Smart organization and keyboard productivity

**Deliver:** nested smart mailbox sources/conditions, cross-mailbox membership, automatic partitions, IMAP keywords with local labels, mailbox-entry rules, bulk operations, mailbox/move chooser, unread/thread navigation, configurable bindings and columns.

Use durable change events for incremental membership. Persist rule-trigger identity so restart/reindex does not blindly re-run actions. Detect dependency loops and bound rule cascades. Add muting only when graph, identities, and moves have defined behavior.

**Exit:** rebuilding a search/thread projection does not duplicate remote actions; saved mailbox results match direct evaluation; multistroke commands and bulk actions work across pages with stable selection.

### Phase 5 — Broader compatibility, automation, and measured scale

**Deliver according to actual need:** more providers/generic IMAP settings, richer Markdown/HTML reply preservation, Send Later, cryptographic verification/signing/decryption, export/backup/repair, a stable command API, and configurable layout/filter extensions.

Multiple-account support should reuse Phase 0/1 identity and credential foundations; its user-facing priority may move earlier if required. Do not advertise a provider merely because its domain/MX is recognized. Keep complex scripting integration behind stable commands rather than exposing storage internals.

Scale/durability measurement runs throughout all phases, not only at the end. This phase is where broader performance or interoperability claims become justified by collected results.

### Suggested first implementation batches

1. **Search/Correspondence correctness:** Cc/name projection migration, exhaustive scoped retrieval, anchor-plus-page response, and regression fixtures.
2. **Protocol identity correctness:** full reply headers and date fields; stable credential identity/lock; successful-LIST generation reconciliation.
3. **Operational daily-driver slice:** special-use folders plus durable archive/trash/move, multiselection, queued/error status, reconnect recovery.
4. **Read/reply slice:** MIME parts and attachments, useful reader links/CID, Reply-All and multiple drafts, independently recoverable Sent copy.

Each batch should remain reviewable and preserve existing data through explicit migrations. They are dependency-oriented batches, not a claim that the entire client can be finished in four small patches.

## Validation plan and acceptance criteria

Use synthetic mail and disposable test accounts; never mutate MailMate/msgvault archives for tests. Add behavior tests at meaningful failure boundaries, rather than snapshots that merely repeat implementation details.

| Area | Essential fixture or experiment | Required outcome |
| --- | --- | --- |
| Folder listing | Successful list loses a folder; separate run fails mid-enumeration | Missing folder retires only after authoritative completion; archive bytes retained |
| UID identity | UID reused under changed UIDVALIDITY; old pending flag/move | No stale action applied to the new message; safe resync |
| Concurrent flags | User changes flag while remote acknowledgment arrives | Newer local intent survives stale acknowledgment/snapshot |
| Incremental sync | Checkpoint reset, disconnect, missing capability, malformed response | Full-reconcile fallback; no checkpoint past durable data |
| Cancellation | Cancel during list/fetch/ingest/action | Bounded stop latency; explicit resumable state; no false completion |
| Large/malformed MIME | Oversized message, deeply nested parts, corrupt transfer encoding | Bounded processing, visible quarantine/skip, next work can continue |
| Move/copy | Crash after destination copy but before source deletion | Recoverable operation; no blind repeated copy or unverified deletion |
| Threading | Missing/late parent, self-loop, A↔B cycle, duplicate RFC ID, long chain | Deterministic bounded graph, no recursion failure, no unrelated collapse |
| Replies | A→B→C reply-to-reply; absent/malformed References | Correct bounded chain and intended thread after ingest |
| Correspondence | A→B, B→A, A→Cc-B, unrelated A→C, old anchor, junk anchor | Correct pair relation; anchor retained; role-aware exclusions; complete pages |
| Scoped search | More than 1,000 higher-ranked out-of-scope/retired hits; Cc-only hit | Valid scoped hits still returned and page to exhaustion |
| Search semantics | Diacritics, quoted/unquoted text, field scopes, Boolean groups, calendar dates | Documented semantics across rebuild and query entry points |
| MIME reader | Multipart alternative/related, missing CID, linked CSS resource, cached image | Correct part choice and missing-resource feedback; no unintended fetch |
| Drafts/envelope | Multiple drafts, display names, Reply-To, duplicate recipients, Bcc | Restart-safe drafts; correct identity/envelope; no Bcc disclosure |
| SMTP | Connect failure, explicit rejection, partial recipients, lost post-DATA reply | Correct certainty classification; no automatic retry of ambiguous recipients |
| Sent | SMTP accepted then append fails; server auto-saves sent message | Sent recovery independent of sending; no duplicate SMTP submission |
| Credential refresh | Two archives share a rotating token; process dies during refresh | Correct shared lock/ownership; recoverable or explicit reconnect state |
| Rules | Reindex/restart, membership entry/exit/reentry, mutually moving rules | Defined trigger replay, bounded cascades, no accidental repeated actions |
| Durability | Kill around blob publication, metadata commit, projection commit/ack | No acknowledged data loss; repairable orphan/projection state |
| UI | Native screenshots and interactions for each delivered workflow | Readable panes, correct focus/selection, usable settings and error/progress states |

For contract/backend changes, follow repository requirements: `just format`, `just ci`, and `just bench-check`. This research-only change does not modify executable code and did not rerun those checks.

Performance measurements should separate MIME extraction, ingest, durable commits, cold/warm search, mailbox/thread/Correspondence pagination, and concurrent sync/index/navigation. Use 100k and 1M **synthetic** metadata/message fixtures with realistic skew, long threads, and cross-folder copies. Report dataset shape, machine, latency percentiles, memory, database size, and query plans. The existing warm 10k search and small ingest/index benchmarks do not establish million-message or real-account performance. Set user-visible latency budgets from measurements; do not infer MailMate's performance from its function names or data structures.

## Open questions and investigation coverage

### Established versus still unresolved

| Topic | Established here | Follow-up needed |
| --- | --- | --- |
| Correspondence | Handler predicate construction and shipped layout semantics | Runtime alias/case handling and cross-account behavior |
| Threads | Parent/child links, keyed candidates, thread-ID updates, cycle-edge removal | Exact header precedence, duplicate-ID policy, missing-parent and removal behavior |
| Persistence | Metadata serialization, staged operation markers, backup/content helpers | Full recovery path, fsync order, power-loss experiments |
| IMAP | Action scheduling, state checks, mailbox schedules, MODSEQ configuration presence | Wire-level capability negotiation, merge/conflict policy, durable checkpoint timing |
| SMTP | Action submission, duplicate internal-message guard, progress/release paths | Ambiguous delivery, partial recipients, retry and Sent-copy algorithms |
| Search | Shipped syntax documentation, aliases, readable conversion example | Actual binary parser edge cases and planner/index behavior |
| Rules | Manual command wrapper; documented mailbox-entry triggers | Native evaluator, event replay, action ordering and cascade handling |
| Identities | Sent-recipient query and From/source fallback paths | Exact ranking, alias-pattern precedence, explicit override interaction |
| Renderer | CID/CSS handling, blocked-resource/cache/presentation paths | Complete URL-policy matrix, redirect handling, runtime cancellation and cache policy |
| Crypto | Documented integrations | No cryptographic implementation or runtime validation in this study |

There are 36,128 analyzed procedures, but only 17 focused dossiers were requested. The broad feature inventory comes primarily from shipped help/configuration; this is not complete decompilation of MailMate. Several large functions were reviewed through characteristic call/literal summaries with full dossiers retained, so exact control-flow claims remain deliberately narrow.

The highest-value further experiments are synthetic three-message reference graphs, mailbox-entry rule transitions, offline move/reconnect, and blocked/cached/CID HTML resources. These require an isolated test profile or disposable account to establish runtime behavior. None was performed against personal mail in this pass.

### Avoid misleading parity claims

- A zero `outbox` procedure-name search is not an absent outbox.
- The downloadable bundle catalog is not the set of installed integrations.
- Old help's WebView description does not supersede newer WKWebView release notes.
- An HTML-commented statement about offline mailbox operations is not current visible help.
- Commented-out AppleScript make/delete/move declarations are not supported active commands.
- A `highest_modseq` literal is not proof of a fully recovered QRESYNC implementation.
- A method named `atomic_saving` is not proof of crash/power-loss atomicity.
- IMAP-account offline state does not necessarily disable SMTP; shipped hidden preferences describe an option controlling that relationship.
- No claim is made that MailMate and Moonmail share a database architecture or have measured performance parity.

## Evidence ledger and reproducibility

### Native evidence

Full retained bundle, outside the repository:

```text
/Users/penso/.local/share/moonmail-research/mailmate-6272-2026-11-03/evidence.json
```

Export at research completion: **34 Evidence records, 4,451,136 bytes**. The exporter reported zero separate unknown records; this does **not** remove the limitations embedded in each dossier. The export contains native pseudocode/assembly and search results, not personal email content. Addresses are analysis addresses for this exact binary/slice, not portable runtime addresses under ASLR.

Overview Evidence: `ev_25147377b26ea2b50f262c37c6fa4017f5f031262ac4d2f61d11a0f48b27aba8`.

Hopper analysis-profile digest: `b64e0d7e4dd1e6ff416d28d6847c00651d9898838c0235b5c64192b91d33dc44`.

| ID | Procedure / address | REA Evidence ID |
| --- | --- | --- |
| D01 | `showCorrespondence:` — `0x100137510` | `ev_ebda986bfbfaaa6470f572f7ed1486e02710ea2380f0aa5eaf3d4a0c206b75b4` |
| D02 | `reference_threading_t::look_for_parents` — `0x100365ba0` | `ev_551c2f28dfbe320da835af4f879bc233ae756de458434c9cef0a15fa120120db` |
| D03 | `reference_threading_t::check_for_cycles` — `0x100365cfc` | `ev_f231fbbcb940efbc0ac7eea7edf101fc3ca04c48a54e18e5d1ab659975bcab0a` |
| D04 | `reference_threading_t::add_msg_ids` — `0x1003661c4` | `ev_47741f84f79ecb118cf6515f6f4bf5d0d0f14547d0eaf6f504f01f4e99396a1d` |
| D05 | `database_t::save` — `0x1002b6de4` | `ev_7702dab0ac441a21d2c682c7f9be3d11c546d542c5fdf1dc98701bd7eee78c7e` |
| D06 | `imap_account_t::synchronize` — `0x1002eff68` | `ev_4106e8cccbbde308336f310558df01a94157b776e89885a1c790b9fd345ac76f` |
| D07 | `imap_account_t::synchronize_account` — `0x1002f4a0c` | `ev_f1d4b2325d6350154415e49f0f4439b4edf38a23b95acfc383628e0db404764f` |
| D08 | `smtp_account_t::synchronize_account` — `0x10037a4a8` | `ev_4655fd881849eba24d9e4b5060c60fbaa9d69f87194d114c0f913444361f01e8` |
| D09 | `imap_account_t::get_synchronization_interval` — `0x1002efc18` | `ev_05940e7847f518257d5a5ccccdbcfef3199d62851adb12d1af332282087710df` |
| D10 | `add_msg_ids` block callback — `0x100366730` | `ev_f8dae76d1e1259b8c95c1541e4db632b81b820dc49ca79a5eeb7d3717251fc67` |
| D11 | `atomic_saving::run_in_background` block 2 — `0x1004afed4` | `ev_f02511a374d684ce6fccfc310c462dc14d49637e52562a2157cbb5c9491dfaeb` |
| D12 | `smtp_account_t::send_message` — `0x10037b50c` | `ev_51bb1c7c085e2fa68b54766f6b0ed12d4e71d01ae4dcb6d82dbd21bd8aa891ff` |
| D13 | `smtp_account_t::did_finish` — `0x10037b2b8` | `ev_4d5cdc504ab0c9237958e2909370855510460f84b2dc0611c226dd347ba5923e` |
| D14 | `identities_manager_t::identity_for_recipients` — `0x1002edde0` | `ev_89a3ce166a826117a995fa8706de7c814716c08d411e09fde5ab4cfe94116325` |
| D15 | `MmCIDSchemeHandler startURLSchemeTask:` — `0x1001e932c` | `ev_c8643774edb150501df3d8f96d9ec1a39fd99b72b783d5e916168e463678ec9e` |
| D16 | `MmBlockHTTPImgSchemeHandler startURLSchemeTask:` — `0x1001eb924` | `ev_5a3e66663d1f9e6d637ed9670a25693cfe54edc65afeda4b64619a0d577ccbf2` |
| D17 | `MmMessagesWindowController applyRules:` — `0x10013af2c` | `ev_f4f47ead6c945cab80e9010b5a799967a67ca65f5251e036181b0ef56576bc88` |

To reproduce or deepen a finding: verify the executable hash, open the arm64 slice with the same provider/profile, search the named symbol, and run REA `analyze_function` on the discovered address. Import the evidence bundle for retained observations; it is an evidence export, not a promise that every analysis query can replay without the binary/provider. Preserve full dossier limitations when citing compact summaries.

### Shipped resource ledger

Prefixes below are absolute directories:

- **R:** `/Applications/MailMate.app/Contents/Resources`
- **H:** `R/English.lproj/MailMate Help`
- **B:** `/Applications/MailMate.app/Contents/SharedSupport/Bundles`

| ID | Files / sections | Supports |
| --- | --- | --- |
| R01 | `R/Layouts/Mailboxes/correspondence.plist:77–87`; `threadArcs.plist:63–66`; `R/specifiers.plist:12–30,147–153`; `R/MmMessageListView/outlineColumns.plist:227–275` | Correspondence union/pairing, recipient aliases, thread expansion, identity-subtracted correspondents |
| R02 | `H/view.html:30–228`; `R/specifiers.plist`; `B/MailMate.mmbundle/Support/bin/generate_filter_script:9–215`; `B/MailMate.mmbundle/Commands/Search.mmCommand` | Search scopes/grammar, dates, script-versus-invoked-binary distinction |
| R03 | `H/organize.html:19–130`; `R/defaultMailboxes.plist`; `R/standardMailboxes.plist:4–104` | Smart mailbox sets/partitions, universal folders, Personal Inbox, tags, entry-trigger rules, muting |
| R04 | `H/compose.html:29–106`; `H/preferences.html:75–149`; `H/account_setup.html:28–65`; `H/hidden_preferences.html:184–194` | Composition, Markdown, original HTML, identities, signatures, Send Later, checks |
| R05 | `H/account_setup.html:15–189`; `H/rebuild.html:14–18`; `H/preferences.html:25–31`; `H/hidden_preferences.html:392–394` | Accounts, subscriptions, schedules, connection states, local data, SMTP/offline distinction |
| R06 | `R/ReleaseNotes.html:731–778`; `H/preferences.html:64–68,197–221`; `R/messagePartConditions.plist:2–16`; `R/tracker-blocklist.txt` | Modern renderer, resource blocking/cache, attachment predicates, crypto documentation |
| R07 | `H/navigate.html:17–128`; `H/custom_key_bindings.html:78–227`; `H/customization.html:14–25,36–61,107–182`; `R/KeyBindings/{Standard,Gmail,Postbox}.plist` | Keyboard workflow, selectors/sequences, overrides, columns/layouts |
| R08 | `R/eventFilters.plist`; `B/MailMate.mmbundle`; `B/Export.mmbundle`; `R/DefaultBundles.tbz` | Filter pipelines, installed commands/themes, export, downloadable catalog |
| R09 | `R/MailMate.sdef:285–377`; `H/extended_url_scheme.html:34–50,84–107`; `H/custom_key_bindings.html:250–259`; `H/emate.html:14–64` | Active AppleScript commands, trusted mailto extensions, selector execution, CLI |
| R10 | `/Applications/MailMate.app/Contents/Info.plist`; `R/ReleaseNotes.html:24–26,4521,4942` | Target identity/beta date; historical cycle and References-threading notes |

Inventory: 17 help HTML pages; 18 layout files including eight mailbox layouts; Standard/Gmail/Postbox bindings; two shipped bundles containing 14 commands, nine filters, and four themes. `DefaultBundles.tbz` contains a **31-entry downloadable catalog**, not 31 installed bundles. Active scripting includes fetch/fetchlist, open mailto, send, open location, and perform; standard make/delete/move/document declarations are commented out. Extended attachment/send-now mailto parameters require a trusted AppleScript path according to help.

### Moonmail source ledger

Paths below are relative to the repository at the baseline commit; line ranges are review anchors, not stable APIs.

| ID | Files / ranges | Supports |
| --- | --- | --- |
| S01 | `crates/moonmail-store/src/archive.rs:31–55,119–248`; `blobs.rs:44–76`; `crates/moonmail-core/src/mime.rs:10–35` | Durability, ingest, addresses, simplified thread key, projection, reconciliation, MIME contract |
| S02 | `crates/moonmail-store/src/queries.rs:11–189`; `archive.rs:260–264` | Live views, metadata-first keyset pages, grouping, Correspondence, address normalization |
| S03 | `crates/moonmail-network/src/incoming.rs:25–160`; `sync.rs:21–95` | IMAP boundaries, full snapshots, flags, UIDVALIDITY, folder/size/failure limits |
| S04 | `crates/moonmail-store/src/operations.rs:8–160`; `crates/moonmail-network/src/submission.rs:20–142` | Drafts, flags, outbox transitions, frozen submission, reply References, uncertainty |
| S05 | `crates/moonmail-network/src/auth.rs:86–97,107–132,156–202,234–303,358–383`; `credentials.rs:25–47`; `crates/moonmail-app/app/components/settings-accounts.luau:23–56` | Fastmail authentication, credential/lock scope, supported-provider distinction |
| S06 | `crates/moonmail-search/src/sqlite.rs:83–106`; `crates/moonmail-core/src/mail.rs:52–95`; `crates/moonmail-store/src/archive.rs:182–199` | Search contract/ranking/projection and Cc omission |
| S07 | `crates/moonmail-app/src/provider.rs:99–112,157–175,196–229,344–355,443–538`; `app/main.luau:37–68,113–124,197–290`; `app/components/` | App query caps, selection, threading, compose, settings and navigation |
| S08 | `crates/moonmail-app/src/page.rs:8–41`; `app/main.luau:217–229`; `README.md:122–149` | Restricted HTML reader, missing attachment/crypto/composition features |
| S09 | `crates/moonmail-app/src/cli.rs:14–29,104–145`; `README.md:98–100,129–149,191–201` | Account/import CLI, declared interoperability and workflow limits |
| S10 | `crates/moonmail-app/src/local.rs:31–47`; `crates/moonmail-app/src/main.rs:35–50`; `crates/moonmail-bench/benches/backends.rs:50–111`; `crates/moonmail-store/tests/archive.rs:39–205`; `crates/moonmail-network/tests/sync.rs:54–98`; `crates/moonmail-search/tests/contract.rs:67–132` | Projection worker, current benchmark scope and existing behavior tests |

Build provenance caveat: `.cargo/config.toml:1–10` overrides Moonframe's Git dependency with `../moonframe-mod-titlebar/crates/*`. This supersedes README's Git-only provenance description. The inspected native-provider threading implementation is in `/Users/penso/code/moonframe-mod-titlebar/crates/moonframe-providers/src/native.rs:23–35,148–165`. Reproducing current native UI behavior therefore requires recording that sibling checkout as well as Moonmail's commit.
