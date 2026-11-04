"""Capture Moonmail's actual GPUI scene with isolated synthetic data.

Native WebKit surfaces are not included in GPUI's render_to_image output.
This is a layout check, not a complete screenshot of the email reader.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("view", choices=("mail", "threads", "compose", "settings", "fonts", "accounts", "add-account", "providers", "outbox", "feedback"), default="mail", nargs="?")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parent.parent
    binary = repo / "target/debug/moonmail"
    output = args.output.resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="moonmail-capture-") as directory:
        root = Path(directory)
        env = dict(os.environ, MOONMAIL_DEMO="1", MOONMAIL_DATA_DIR=str(root),
                   XDG_CONFIG_HOME=str(root / "config"), XDG_CACHE_HOME=str(root / "cache"),
                   XDG_STATE_HOME=str(root / "state"))
        env.pop("MOONFRAME_CAPTURE", None)
        subprocess.run([binary, "seed-demo"], env=env, check=True, timeout=30)
        if args.view == "threads":
            (root / "preferences.json").write_text(json.dumps({"threaded": True}))
        if args.view == "compose":
            with sqlite3.connect(root / "archive.sqlite3") as db:
                db.execute("INSERT INTO drafts VALUES (?,?,?,?,?,unixepoch())",
                           ("visual-check", "clara@example.org", "A quieter place to work",
                            "Hi Clara,\n\nThanks for sending your notes. Let's talk on Thursday.\n\nYou", ""))
        overlay = root / "overlay"
        overlay.mkdir()
        source = (repo / "crates/moonmail-app/app/main.luau").read_text()
        marker = 'pane = "mail", rich ='
        if source.count(marker) != 1:
            raise RuntimeError("Update the capture harness for the shell's initial view")
        settings_views = ("settings", "fonts", "accounts", "add-account", "providers")
        pane = "settings" if args.view in settings_views else "mail" if args.view in ("feedback", "threads") else args.view
        if args.view in settings_views:
            source = source.replace("width = 1500, height = 940", "width = 980, height = 740")
            source = source.replace("local mail = app.mail or {}", 'local mail = app.mail or {}\n\t\tif state.pane == "settings" then return ui.mount "settings" end', 1)
            settings = (repo / "crates/moonmail-app/app/mods/settings/main.luau").read_text()
            if args.view == "fonts":
                settings = settings.replace('page = "appearance", values', 'page = "fonts", values', 1)
                settings = settings.replace('local mail = msg.value or {}', '''local mail = msg.value or {}
                if not state.capture_fonts then
                    state.capture_fonts = true
                    return open_fonts(state)
                end''', 1)
            elif args.view != "settings":
                settings = settings.replace('page = "appearance", values', 'page = "account", values', 1)
                if args.view in ("add-account", "providers"):
                    settings = settings.replace('add_account = false, provider_list = false', 'add_account = true, provider_list = ' + ('true' if args.view == 'providers' else 'false'), 1)
            mod = overlay / "mods/settings"
            mod.mkdir(parents=True)
            (mod / "main.luau").write_text(settings)
            shutil.copytree(repo / "crates/moonmail-app/app/mods/settings/icons", mod / "icons")
        if args.view == "threads":
            source = source.replace("return mod {", 'return mod {\n\tsubscribe = { "provider/mail" },', 1)
            source = source.replace("update = function(state, msg)", '''update = function(state, msg)
        if type(msg) == "table" and msg.topic == "provider/mail" and not state.capture_expanded and msg.value and msg.value.rows and msg.value.rows[1] then
            state.capture_expanded = true
            return state, { { to = "provider:mail", msg = { action = "expand", id = msg.value.rows[1].id } } }
        end''', 1)
        (overlay / "main.luau").write_text(source.replace(marker, f'pane = "{pane}", rich =', 1))
        if args.view == "feedback":
            metadata = json.loads(subprocess.check_output(
                ["cargo", "metadata", "--locked", "--format-version=1"], cwd=repo, text=True))
            package = next(p for p in metadata["packages"] if p["name"] == "moonframe")
            builtin = Path(package["manifest_path"]).parents[2] / "app/mods/feedback/main.luau"
            feedback = builtin.read_text()
            initial = "\t\t\topen = false,"
            if feedback.count(initial) != 1:
                raise RuntimeError("Update the capture harness for Moonframe's feedback state")
            mod = overlay / "mods/feedback"
            mod.mkdir(parents=True)
            (mod / "main.luau").write_text(feedback.replace(initial, "\t\t\topen = true,", 1))
        env["MOONFRAME_CAPTURE"] = str(output)
        subprocess.run([binary, "--user", overlay], env=env, check=True, timeout=60)
    if not output.is_file():
        raise RuntimeError("No capture produced; build with --features capture")
    print(f"GPUI layout capture: {output} (native WebKit surface excluded)")


if __name__ == "__main__":
    main()
