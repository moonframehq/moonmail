"""Capture Moonmail's actual GPUI scene with isolated synthetic data.

Native WebKit surfaces are not included in GPUI's render_to_image output.
This is a layout check, not a complete screenshot of the email reader.
"""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import struct
import tempfile
import zlib


def luau(value):
    """Serialize a JSON provider snapshot as a Luau literal, including UTF-8."""
    if value is None:
        return "nil"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return '"' + ''.join(f"\\{byte:03d}" for byte in value.encode()) + '"'
    if isinstance(value, list):
        return "{" + ",".join(luau(item) for item in value) + "}"
    if isinstance(value, dict):
        return "{" + ",".join(f"[{luau(key)}]={luau(item)}" for key, item in value.items()) + "}"
    return str(value)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path)
    parser.add_argument("view", choices=("mail", "threads", "thread-warning", "unread-navigation", "correspondence", "searches", "search-results", "partitions", "mailbox-chooser", "move-chooser", "copy-chooser", "folders", "tags", "rules", "columns", "column-settings", "export", "compose", "from-menu", "forward", "drafts", "attachments", "links", "images", "identities", "identity-compose", "settings", "fonts", "sync-settings", "accounts", "add-account", "providers", "gmail-setup", "imap-setup", "outbox", "feedback", "organize", "operations"), default="mail", nargs="?")
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
        if args.view == "thread-warning":
            references = " ".join(f"<ancestor-{n}@example.org>" for n in range(300))
            fixture = root / "thread-warning.eml"
            fixture.write_text(f"From: Clara <clara@example.org>\nTo: you@example.com\nSubject: Synthetic long reference chain\nMessage-ID: <long-chain@example.org>\nReferences: {references}\n\nOriginal headers remain complete.\n")
            subprocess.run([binary, "import", fixture], env=env, check=True, timeout=30)
            with sqlite3.connect(root / "archive.sqlite3") as db:
                db.execute("UPDATE messages SET date=1900000000 WHERE subject='Synthetic long reference chain'")
        if args.view in ("correspondence", "threads", "unread-navigation"):
            fixtures = root / "related-fixtures"
            fixtures.mkdir()
            for number in range(125 if args.view == "correspondence" else 237):
                (fixtures / f"{number}.eml").write_text(f"From: Clara <clara@example.org>\nTo: you@example.com\nSubject: Synthetic history {number:03d}\nMessage-ID: <history-{number}@example.org>\nReferences: <history-root@example.org>\n\nSynthetic correspondence paging fixture.\n")
            subprocess.run([binary, "import", fixtures], env=env, check=True, timeout=30)
            with sqlite3.connect(root / "archive.sqlite3") as db:
                db.execute("UPDATE messages SET date=1900000000+id WHERE subject LIKE 'Synthetic history %'")
                if args.view == "unread-navigation":
                    db.execute("UPDATE occurrences SET seen=1")
                    db.execute("UPDATE occurrences SET seen=0 WHERE message_id=(SELECT id FROM messages WHERE subject LIKE 'Synthetic history %' ORDER BY date,id LIMIT 1 OFFSET 180)")
        if args.view in ("links", "images"):
            def chunk(kind, data):
                return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
            pixels = b"".join(b"\0" + bytes((50, 120, 210)) * 160 for _ in range(80))
            png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 160, 80, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(pixels)) + chunk(b"IEND", b"")
            fixture = root / "reader.eml"
            fixture.write_text('From: Clara <clara@example.org>\nTo: you@example.com\nSubject: Synthetic reader resources\nMIME-Version: 1.0\nContent-Type: multipart/related; boundary=reader\n\n--reader\nContent-Type: text/html; charset=utf-8\n\n<h2>Review notes</h2><p>See <a href="https://example.org/design">the design notes</a> and <a href="https://example.net/review?week=44&amp;view=summary">review summary</a>.</p><img src="cid:preview@example.org" alt="Synthetic blue preview"><img src="https://tracker.example/pixel">\n--reader\nContent-Type: image/png\nContent-ID: <preview@example.org>\nContent-Transfer-Encoding: base64\n\n' + base64.b64encode(png).decode() + '\n--reader--\n')
            subprocess.run([binary, "import", fixture], env=env, check=True, timeout=30)
            with sqlite3.connect(root / "archive.sqlite3") as db:
                db.execute("UPDATE messages SET date=(SELECT MAX(date)+1 FROM messages) WHERE subject='Synthetic reader resources'")
        if args.view in ("attachments", "forward"):
            subprocess.run([binary, "import", repo / "crates/moonmail-mime/tests/fixtures/attachments.eml"], env=env, check=True, timeout=30)
            with sqlite3.connect(root / "archive.sqlite3") as db:
                db.execute("UPDATE messages SET date=(SELECT MAX(date)+1 FROM messages) WHERE subject='Synthetic attachment review'")
        if args.view in ("threads", "unread-navigation"):
            (root / "preferences.json").write_text(json.dumps({"threaded": True, "mark_read": False}))
        if args.view == "columns":
            (root / "preferences.json").write_text(json.dumps({"threaded": True, "columns": [
                {"field": "date", "width": 156, "visible": True},
                {"field": "subject", "width": 0, "visible": True},
                {"field": "recipients", "width": 220, "visible": True},
                {"field": "sender", "width": 220, "visible": False}]}))
        if args.view in ("compose", "drafts", "from-menu"):
            with sqlite3.connect(root / "archive.sqlite3") as db:
                for number, subject in enumerate(("Thursday's design review", "Notes for the team", "Re: A quieter place to work")):
                    db.execute("INSERT INTO drafts(id,recipients,cc,bcc,account,subject,body,reply_id,updated) VALUES (?,?,?,?,?,?,?,?,?)",
                               (f"visual-check-{number}", '"Clara Chen" <clara@example.org>',
                                '"Design team" <design@example.org>', "private@example.org", "you@example.com", subject,
                                "Hi Clara,\n\nThanks for sending your notes. Let's talk on Thursday.\n\nYou\n\nOn Monday, Clara wrote:\n> A little more space for focused work would help.\n> What do you think?",
                                "parent@example.org" if number == 2 else "", 1804000000000 + number))
                attachments = []
                for number, (name, content) in enumerate((("design-notes.txt", b"Synthetic design notes\n"), ("sample.bin", bytes(range(256))))):
                    digest = hashlib.sha256(content).hexdigest()
                    blob = root / "attachments" / digest[:2] / f"{digest}.bin"
                    blob.parent.mkdir(parents=True, exist_ok=True)
                    blob.write_bytes(content)
                    attachments.append({"id": f"capture-{number}", "name": name, "blob": digest, "bytes": len(content)})
                db.execute("UPDATE drafts SET attachments_json=? WHERE id='visual-check-2'", (json.dumps(attachments),))
        if args.view == "operations":
            with sqlite3.connect(root / "archive.sqlite3") as db:
                destination = db.execute("SELECT id FROM mailboxes WHERE name='Trash'").fetchone()[0]
                messages = db.execute("SELECT o.message_id,o.mailbox_id FROM occurrences o JOIN mailboxes b ON b.id=o.mailbox_id WHERE b.name='INBOX' LIMIT 3").fetchall()
                for uid, ((message, source), state) in enumerate(zip(messages, ("queued", "unknown", "failed")), 1):
                    db.execute("UPDATE occurrences SET epoch=7,uid=? WHERE message_id=? AND mailbox_id=?", (uid, message, source))
                    detail = {"queued": "Waiting for next Sync", "unknown": "Response lost. Sync and inspect both folders before dismissing.", "failed": "Destination folder disappeared; choose an active folder."}[state]
                    db.execute("INSERT INTO move_operations(message_id,source_id,destination_id,epoch,uid,state,detail) VALUES (?,?,?,7,?,?,?)", (message, source, destination, uid, state, detail))
        if args.view == "tags":
            with sqlite3.connect(root / "archive.sqlite3") as db:
                message, mailbox = db.execute("SELECT o.message_id,o.mailbox_id FROM occurrences o JOIN messages m ON m.id=o.message_id ORDER BY m.date DESC,m.id DESC LIMIT 1").fetchone()
                db.execute("UPDATE occurrences SET epoch=7,uid=1 WHERE message_id=? AND mailbox_id=?", (message, mailbox))
                db.execute("INSERT INTO keyword_operations(mailbox_id,epoch,uid,keyword,value,state,detail) VALUES (?,7,1,'$label1',1,'blocked','Server does not permit this permanent keyword')", (mailbox,))
                db.execute("INSERT INTO occurrence_keywords VALUES (?,?,'$label1')", (mailbox, message))
        if args.view == "outbox":
            with sqlite3.connect(root / "archive.sqlite3") as db:
                for number, (state, detail) in enumerate((("cancelled", "Returned to drafts"), ("sent", "Accepted by SMTP server"), ("unknown", "Connection lost after submission began. Check Sent before composing a replacement."), ("failed", "Connection failed before submission; an explicit retry is available."), ("queued", "Waiting for an explicit Send"))):
                    draft = {"id": f"outbox-capture-{number}", "account": "you@example.com", "recipients": "Clara <clara@example.org>", "subject": f"Synthetic {state} message", "body": "Synthetic saved body", "attachments": []}
                    db.execute("INSERT INTO outbox(draft_id,account,blob,sender,recipients,subject,state,detail,created,draft_json) VALUES (?,?,?,?,?,?,?,?,?,?)", (draft["id"], draft["account"], "synthetic-unused", draft["account"], json.dumps(["clara@example.org"]), draft["subject"], state, detail, number, json.dumps(draft)))
                message, mailbox, rfc_id = db.execute("SELECT m.id,b.id,m.rfc_id FROM messages m JOIN occurrences o ON o.message_id=m.id JOIN mailboxes b ON b.id=o.mailbox_id WHERE b.account='you@example.com' AND b.name='Sent' LIMIT 1").fetchone()
                db.execute("UPDATE occurrences SET uid=1,epoch=7 WHERE message_id=? AND mailbox_id=?", (message, mailbox))
                db.execute("UPDATE outbox SET rfc_id=? WHERE state='sent'", (rfc_id,))
                db.execute("UPDATE outbox SET rfc_id='missing@moonmail.local' WHERE state='unknown'")
        if args.view in ("identities", "identity-compose"):
            with sqlite3.connect(root / "archive.sqlite3") as db:
                db.execute("INSERT INTO identities VALUES (?,?,?,?,?,?,?)", ("work", "you@example.com", "You · Design", "design@example.org", "replies@example.org", "You\nDesign team\nexample.org", 1))
                db.execute("INSERT INTO identities VALUES (?,?,?,?,?,?,?)", ("personal", "you@example.com", "You", "you@example.com", "", "You", 0))
        overlay = root / "overlay"
        overlay.mkdir()
        source = (repo / "crates/moonmail-app/app/main.luau").read_text()
        marker = 'pane = "mail", rich ='
        if source.count(marker) != 1:
            raise RuntimeError("Update the capture harness for the shell's initial view")
        settings_views = ("settings", "fonts", "sync-settings", "column-settings", "accounts", "add-account", "providers", "gmail-setup", "imap-setup")
        pane = "settings" if args.view in settings_views else "mail" if args.view in ("feedback", "threads", "thread-warning", "unread-navigation", "images", "correspondence", "search-results", "columns") else "compose" if args.view in ("forward", "identity-compose") else args.view
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
            elif args.view == "sync-settings":
                settings = settings.replace('page = "appearance", values', 'page = "sync", values', 1)
            elif args.view == "column-settings":
                settings = settings.replace('page = "appearance", values', 'page = "reading", values', 1)
                snapshot = json.loads(subprocess.check_output([binary, "capture-snapshot", "mail"], env=env, text=True, timeout=30))
                settings = settings.replace("local mail = app.mail or {}", "local mail = " + luau(snapshot), 1)
            elif args.view != "settings":
                settings = settings.replace('page = "appearance", values', 'page = "account", values', 1)
                snapshot = json.loads(subprocess.check_output([binary, "capture-snapshot", args.view], env=env, text=True, timeout=30))
                settings = settings.replace("local mail = app.mail or {}", "local mail = " + luau(snapshot), 1)
                if args.view in ("gmail-setup", "imap-setup"):
                    provider = "google" if args.view == "gmail-setup" else "other"
                    email = "you@gmail.com" if provider == "google" else "you@example.org"
                    setup = 'state.add_account = true; state.email = ' + luau(email) + '; state.account_presets = mail.account_presets; choose_provider(state, ' + luau(provider) + ')'
                    settings = settings.replace("local v = state.values", setup + "\n\t\tlocal v = state.values", 1)
                if args.view in ("add-account", "providers"):
                    settings = settings.replace('add_account = false, provider_list = false', 'add_account = true, provider_list = ' + ('true' if args.view == 'providers' else 'false'), 1)
            mod = overlay / "mods/settings"
            mod.mkdir(parents=True)
            (mod / "main.luau").write_text(settings)
            shutil.copytree(repo / "crates/moonmail-app/app/mods/settings/icons", mod / "icons")
        if args.view not in settings_views and args.view != "feedback":
            snapshot = json.loads(subprocess.check_output([binary, "capture-snapshot", args.view], env=env, text=True, timeout=30))
            if args.view in ("compose", "forward", "identity-compose", "from-menu"):
                # Render the real composer mod at its own native window size,
                # rather than stretching it inside the main mailbox shell.
                source = (repo / "crates/moonmail-app/app/mods/mail-composer/main.luau").read_text()
                source = source.replace('min_size = { width = 620, height = 520 }, ', '', 1)
                marker = None
            source = source.replace("local mail = app.mail or {}", "local mail = " + luau(snapshot), 1)
            if args.view in ("compose", "forward", "identity-compose", "from-menu"):
                source = source.replace("mail = app.mail or {}", "mail = " + luau(snapshot), 1)
            if args.view == "from-menu":
                source = source.replace("return composer { state = state,", "state.from_open = true; state.from_index = 1\n\t\treturn composer { state = state,", 1)
            if args.view in ("searches", "search-results"):
                source = source.replace('query = "", email = "",', 'query = ' + luau(snapshot["search"]) + ', email = "",', 1)
            if args.view in ("mailbox-chooser", "move-chooser", "copy-chooser"):
                source = source.replace('query = "", email = "",', 'query = "", email = "", chooser_query = "Projects", chooser_index = 2,', 1)
            if args.view == "identities":
                source = source.replace('query = "", email = "",', 'query = "", email = "", identity_edit = ' + luau(snapshot["identities"][0]) + ',', 1)
            if args.view == "tags":
                source = source.replace('query = "", email = "",', 'query = "", email = "", tag_keyword = "project-work", tag_label = "Work projects",', 1)
            if args.view == "rules":
                source = source.replace('query = "", email = "",', 'query = "", email = "", rule_edit = ' + luau(snapshot["keyword_rules"][0]) + ',', 1)
            for kind in ("html_url", "plain_url"):
                reader = root / f"capture-{kind}.html"
                if reader.exists():
                    shutil.copyfile(reader, output.with_suffix(f".{kind}.html"))
        if args.view == "images":
            source = source.replace('rich = false', 'rich = true', 1)
        (overlay / "main.luau").write_text(source.replace(marker, f'pane = "{pane}", rich =', 1) if marker else source)
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
