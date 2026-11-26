#!/usr/bin/env python3
"""Start an isolated loopback JMAP server and run Moonmail's two-user test."""
import base64
import hashlib
import json
import os
from pathlib import Path
import platform
import secrets
import shutil
import socket
import subprocess
import tarfile
import tempfile
import time
from urllib.error import URLError
from urllib.parse import urlencode
from urllib.request import Request, urlopen
import zipfile

RELEASE = "v0.15.5"
ASSETS = {
    ("Darwin", "arm64"): ("aarch64-apple-darwin", "cac0eab57a5c5844efa7e4bc206842a5532af11c9f5ff7cf023775bcdbf4b33a"),
    ("Linux", "x86_64"): ("x86_64-unknown-linux-gnu", "38ea325845c6de77e062d3295b107d01cb804205de1f8186a78f6ffdec1a2832"),
}
DOMAIN = "moonmail.example.org"


def binary(root):
    if path := os.environ.get("STALWART_BIN"):
        return str(Path(path).resolve(strict=True))
    target, expected = ASSETS[(platform.system(), platform.machine())]
    asset = f"stalwart-{target}.tar.gz"
    url = f"https://github.com/stalwartlabs/stalwart/releases/download/{RELEASE}/{asset}"
    path = root / asset
    with urlopen(url, timeout=90) as response, path.open("wb") as out:
        shutil.copyfileobj(response, out)
    if hashlib.sha256(path.read_bytes()).hexdigest() != expected:
        raise RuntimeError("Stalwart download checksum mismatch")
    with tarfile.open(path) as archive, (root / "stalwart").open("wb") as out:
        shutil.copyfileobj(archive.extractfile("stalwart"), out)
    (root / "stalwart").chmod(0o700)
    return str(root / "stalwart")


def request(url, *, auth=None, body=None, form=None):
    headers = {}
    if auth:
        headers["Authorization"] = "Basic " + base64.b64encode(auth.encode()).decode()
    data = None
    if form is not None:
        data = urlencode(form).encode()
        headers["Content-Type"] = "application/x-www-form-urlencoded"
    elif body is not None:
        data = json.dumps(body).encode()
        headers["Content-Type"] = "application/json"
    with urlopen(Request(url, data=data, headers=headers), timeout=5) as response:
        return json.load(response)


def provision(base, admin, user):
    address = f"{user}@{DOMAIN}"
    password = secrets.token_urlsafe(32)
    principal = {"type": "individual", "name": address, "emails": [address],
                 "roles": ["user"], "secrets": [password]}
    created = request(base + "/api/principal", auth=admin, body=principal)
    if "data" not in created:
        raise RuntimeError("Could not create fixture account")
    client_id = "moonmail-test"
    device = request(base + "/auth/device", form={"client_id": client_id})
    approval = request(base + "/api/oauth", auth=f"{address}:{password}",
                       body={"type": "device", "code": device["user_code"]})
    if approval.get("data") is not True:
        raise RuntimeError("Fixture device grant was rejected")
    grant = request(base + "/auth/token", form={
        "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        "client_id": client_id, "device_code": device["device_code"],
    })
    return grant["access_token"]


def run(root):
    executable = binary(root)
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    base = f"http://127.0.0.1:{port}"
    password = secrets.token_urlsafe(32)
    with zipfile.ZipFile(root / "web.zip", "w") as web:
        web.writestr("index.html", "Moonmail test")
    config = root / "config.toml"
    config.write_text(f'''
server.hostname = "127.0.0.1"
server.listener.http.bind = "127.0.0.1:{port}"
server.listener.http.protocol = "http"
storage.data = "db"
storage.fts = "db"
storage.blob = "db"
storage.lookup = "db"
storage.directory = "internal"
store.db.type = "sqlite"
store.db.path = "{root}/mail.sqlite3"
directory.internal.type = "internal"
directory.internal.store = "db"
authentication.fallback-admin.user = "admin"
authentication.fallback-admin.secret = "{password}"
email.folders.archive.name = "Archived mail"
spam-filter.enable = false
version.spam-filter = "1.0.0"
webadmin.resource = "file://{root}/web.zip"
''')
    proc = subprocess.Popen([executable, "--config", str(config)], cwd=root,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    try:
        deadline = time.monotonic() + 45
        while True:
            try:
                request(base + "/.well-known/oauth-authorization-server")
                break
            except URLError:
                if proc.poll() is not None or time.monotonic() > deadline:
                    raise RuntimeError("Stalwart did not start on loopback") from None
                time.sleep(0.2)
        admin = f"admin:{password}"
        created = request(base + "/api/principal", auth=admin, body={"type": "domain", "name": DOMAIN})
        if "data" not in created:
            raise RuntimeError("Could not create synthetic domain")
        env = os.environ.copy()
        env["MOONMAIL_STALWART_URL"] = base + "/jmap/session"
        for user in ("alice", "bob"):
            env[f"MOONMAIL_STALWART_{user.upper()}_TOKEN"] = provision(base, admin, user)
        print("Stalwart ready. Two synthetic accounts have independent bearer grants.", flush=True)
        subprocess.run(["cargo", "test", "--locked", "-p", "moonmail-network", "--test",
                        "stalwart", "--", "--include-ignored",
                        "--nocapture"], check=True, env=env)
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait()


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="moonmail-stalwart-") as temp:
        run(Path(temp).resolve())
