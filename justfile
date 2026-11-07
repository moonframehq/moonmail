nightly_toolchain := "nightly-2026-06-20"

default:
    @just --list

# Runs in the macOS login session, which the Keychain needs: see scripts/run-gui.sh.
run *args:
    scripts/run-gui.sh {{args}}

# `just run` with account and Keychain debug logs in target/moonmail-debug.log.
debug *args:
    RUST_LOG=error,moonframe=info,moonmail=debug MOONMAIL_RUN_LOG=target/moonmail-debug.log scripts/run-gui.sh {{args}}

demo:
    MOONMAIL_DEMO=1 cargo run --locked -p moonmail-app

build:
    cargo build --locked -p moonmail-app --release

check:
    cargo run --locked -p moonmail-app -- --check --no-user

# Capture the real GPUI layout with isolated demo data; excludes native WebKit.
screenshot output="target/screenshots/moonmail.png" view="mail":
    cargo build --locked -p moonmail-app --features capture
    python3 scripts/capture.py {{quote(output)}} {{quote(view)}}

login email:
    scripts/run-gui.sh login {{quote(email)}}

password email:
    cargo run --locked -p moonmail-app -- password {{quote(email)}}

sync:
    scripts/run-gui.sh sync

import path:
    cargo run --locked -p moonmail-app -- import {{quote(path)}}

reindex:
    cargo run --locked -p moonmail-app -- reindex

# Render the app icons: 1024 px for the Dock and bundles, small previews for Settings.
icon:
    for name in wax-seal moon-at orbit; do \
        rsvg-convert -w 1024 -h 1024 assets/icons/$name.svg -o crates/moonmail-app/icons/$name.png && \
        rsvg-convert -w 160 -h 160 assets/icons/$name.svg -o crates/moonmail-app/app/mods/settings/icons/$name.png || exit; \
    done

format:
    cargo +{{nightly_toolchain}} fmt --all
    taplo fmt

format-check:
    cargo +{{nightly_toolchain}} fmt --all -- --check
    taplo fmt --check

ci: format-check
    cargo clippy --locked --workspace --all-targets -- -D warnings
    cargo test --locked --workspace

bench *args:
    cargo bench --locked -p moonmail-bench --bench backends -- {{args}}

bench-check:
    cargo bench --locked -p moonmail-bench --bench backends -- --test

# Native daemon; fixed local ports unless --ephemeral-ports is supplied.
test-maild *args:
    cargo run --locked -p moonmail-test-maild -- {{args}}

# Starts disposable native listeners automatically; no Docker or credentials.
test-mail:
    cargo test --locked -p moonmail-test-maild
    cargo test --locked -p moonmail-network --test local_mail

test-mail-image:
    docker build -f docker/test-maild.Dockerfile -t moonmail-test-maild:local .

test-mail-docker: test-mail-image
    python3 scripts/test_maild_docker.py
