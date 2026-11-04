# Moonmail engineering guide

Moonmail is a local-first Rust email client planned around Moonframe and
MailMate-style Correspondence. The repository includes a runnable Moonframe app,
local archive, MIME/search adapters, Fastmail workers, and comparison harness.

## Architecture

- `moonmail-core`: backend-neutral data and behavioral contracts. No UI, parser,
  database, network, or executor dependencies.
- `moonmail-mime`: interchangeable `mail-parser` and `mailparse` adapters.
- `moonmail-search`: SQLite FTS5 implementation of the search contract.
- `moonmail-bench`: synthetic corpus and generic comparison harnesses.
- `moonmail-store`: authoritative SQLite metadata, immutable blobs, durable journal/outbox.
- `moonmail-network`: generic incoming/submission/credential boundaries, Fastmail adapters.
- `moonmail-app`: CLI, Moonframe provider, Luau shell and bounded UI snapshots.
- The chosen IMAP dependency is exactly `3.0.0-alpha.15` of `imap` (rust-imap).
  The blocking connection adapter owns its socket on a dedicated worker.

## Traits and generics

- Use narrow traits at replaceable behavior boundaries: MIME extraction, blob
  storage, transactional metadata storage, search, incoming mail, submission,
  and credential storage as those components are implemented.
- Prefer static dispatch using named generics or `impl Trait`. Keep concrete
  types at the composition root. Use `dyn Trait` only for actual heterogeneous
  runtime collections or a UI boundary that requires type erasure.
- Prefer existing traits (`Read`, `Write`, `AsRef<Path>`, `IntoIterator`) and
  closures over custom wrappers. Domain data should remain concrete structs and
  enums, not traits for every field or getter.
- Associated error types preserve each implementation's structured errors.
  Do not convert errors to strings or `anyhow` inside library boundaries.
- Define the semantics before adding an alternative: ordering, tokenization,
  atomicity, durability, cancellation, and visibility must be explicit. A trait
  is not interchangeable if each implementation silently changes the operation.
- Avoid a universal storage trait. Authoritative metadata transactions differ
  from rebuildable search projections. Preserve atomic metadata/checkpoint writes;
  use a transactional change log for a separately committed search engine.
- Blocking IMAP/SQLite belongs on connection-owning workers, never the UI thread
  or an async executor thread. Generics do not change that execution requirement.

## Performance and correctness

- Benchmark alternatives through the same generic harness on the same corpus.
  Verify semantic parity outside timed loops. Separate extraction, ingest, search,
  durable disk commits, and in-memory microbenchmarks.
- Keep message bodies out of navigation queries. Page IDs before enrichment;
  use bounded caches, keyset pagination, and lazy body loading.
- Build indexes in bounded transactions; make rebuild progress explicit. Never
  advance a sync/index cursor beyond durable completed work.
- Correspondence is bidirectional sender/recipient matching, distinct from
  References/In-Reply-To threading.
- Use synthetic fixtures. Never change MailMate/msgvault archives in tests or
  benchmarks, and never include real message contents in the repository.

## Rust and tooling

- Centralize dependency versions and lints in the workspace; inherit in crates.
- Keep `lib.rs` for module wiring and exports. Keep modules small and focused.
- Use `thiserror` for typed errors. No production `unwrap`, `expect`, or unsafe.
- Use the pinned stable compiler for builds; the pinned nightly only for rustfmt.
- Run `just format`, `just ci`, and `just bench-check` when changing contracts or
  backend implementations. Add behavior tests for meaningful failure boundaries.
- Document limitations honestly; do not present synthetic benchmark results as
  real-account or million-message performance measurements.
