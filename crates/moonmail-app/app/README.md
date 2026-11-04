# Moonmail's live UI

This is the app tree of a native Rust application, not a standalone Moonframe
profile. Read the repository's `AGENTS.md` and `README.md` before making changes.

- `main.luau` defines the shell and mailbox/Correspondence/reader layout.
- `style.luau` defines the app's colors and styling. Prefix mail-table classes
  with `mail-` to avoid inheriting Moonframe component styles.
- `mods/settings/main.luau` is the standalone Settings window (⌘,).
- `providers/mail` connects to the Rust provider in `../src/provider.rs`.
- `providers/acp` anchors Make it yours agents in the Moonmail repository.

Luau edits reload in the running development app. Rust edits require a rebuild
and restart; do not stop the user's app automatically.

Run commands from the repository root:

```sh
just check
cargo test --locked -p moonmail-app
just screenshot target/screenshots/mail.png mail
```

The screenshot command's second argument is a view (`mail`, `compose`, `settings`,
`outbox`, `threads`, or `feedback`), not a process ID. It launches an isolated synthetic-data window;
the export excludes native WebKit. Inspect that limitation honestly. This repo
does not use Moonframe's `check-examples` or profile-linking commands.
