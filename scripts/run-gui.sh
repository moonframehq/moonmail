#!/bin/sh
# Builds Moonmail and runs it in the macOS login (Aqua) session. A shell under
# SSH or a terminal multiplexer's server is in a Background session, where the
# Keychain fails with "User interaction is not allowed", so there the binary
# starts through a small launcher app: LaunchServices opens apps in Aqua.
# Commands that prompt on the terminal (`password`, `connect`) need a Terminal
# window. MOONMAIL_RUN_LOG sends output to a file instead of this terminal.
set -eu

cargo build --locked -p moonmail-app
root=$(pwd)
binary=$root/target/debug/moonmail

if [ "$(uname)" != Darwin ] || [ "$(launchctl managername)" = Aqua ]; then
    if [ -n "${MOONMAIL_RUN_LOG-}" ]; then
        exec "$binary" "$@" >"$MOONMAIL_RUN_LOG" 2>&1
    fi
    exec "$binary" "$@"
fi

app=$root/target/debug/Moonmail-Launcher.app
pid_file=$root/target/debug/moonmail-launcher.pid
mkdir -p "$app/Contents/MacOS"
cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>launcher</string>
<key>CFBundleIdentifier</key><string>dev.moonmail.launcher</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>LSUIElement</key><true/>
</dict></plist>
PLIST
cat >"$app/Contents/MacOS/launcher" <<LAUNCHER
#!/bin/sh
cd "$root" || exit 1
echo \$\$ >"$pid_file"
exec "$binary" "\$@"
LAUNCHER
chmod +x "$app/Contents/MacOS/launcher"

output=${MOONMAIL_RUN_LOG:-$(tty 2>/dev/null || echo /dev/null)}
case $output in
/*) ;;
*) output=$root/$output ;;
esac
[ -z "${MOONMAIL_RUN_LOG-}" ] || : >"$output"

set -- "$app" --args "$@"
for name in RUST_LOG MOONMAIL_DEMO MOONMAIL_DATA_DIR PATH; do
    if eval "[ -n \"\${$name+set}\" ]"; then
        eval "value=\$$name"
        set -- --env "$name=$value" "$@"
    fi
done

# `open` returning does not stop the app, so Ctrl-C has to.
trap 'kill "$(cat "$pid_file")" 2>/dev/null || true' INT TERM
open -W -n --stdout "$output" --stderr "$output" "$@"
