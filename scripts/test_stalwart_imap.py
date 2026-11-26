#!/usr/bin/env python3
"""Exercise Moonmail IMAP and SMTP over both TLS modes on a disposable Stalwart."""
from contextlib import ExitStack
import json
from pathlib import Path
import socket
import subprocess
import tempfile

from test_stalwart import run


def configure(root):
    # Keep the sockets reserved until all distinct loopback ports have been chosen.
    endpoints = {}
    listeners = []
    with ExitStack() as stack:
        for name in ("imap_tls", "smtp_tls", "imap_starttls", "smtp_starttls"):
            sock = stack.enter_context(socket.socket())
            sock.bind(("127.0.0.1", 0))
            port = endpoints[name] = sock.getsockname()[1]
            prefix = "server.listener." + name
            listeners.extend([f'{prefix}.bind = "127.0.0.1:{port}"',
                              f'{prefix}.protocol = "{name.split("_")[0]}"',
                              f'{prefix}.tls.implicit = {str(name.endswith("_tls")).lower()}'])
    (root / "ready.json").write_text(json.dumps(endpoints))

    def openssl(*args):
        subprocess.run(["openssl", *args], cwd=root, check=True, capture_output=True)

    openssl("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
            "-subj", "/CN=Moonmail fixture CA", "-keyout", "ca.key", "-out", "ca.pem",
            "-addext", "basicConstraints=critical,CA:TRUE",
            "-addext", "keyUsage=critical,keyCertSign,cRLSign")
    openssl("req", "-new", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost",
            "-keyout", "server.key", "-out", "server.csr")
    (root / "leaf.ext").write_text("subjectAltName=DNS:localhost\nextendedKeyUsage=serverAuth\n"
                                   "basicConstraints=critical,CA:FALSE\n")
    openssl("x509", "-req", "-in", "server.csr", "-CA", "ca.pem", "-CAkey", "ca.key",
            "-CAcreateserial", "-days", "2", "-extfile", "leaf.ext", "-out", "server.pem")
    openssl("x509", "-in", "ca.pem", "-outform", "DER", "-out", "ca.der")
    listeners.extend([f'certificate.default.cert = "%{{file:{root}/server.pem}}%"',
                      f'certificate.default.private-key = "%{{file:{root}/server.key}}%"'])
    return "\n".join(listeners)


if __name__ == "__main__":
    with tempfile.TemporaryDirectory(prefix="moonmail-stalwart-imap-") as directory:
        root = Path(directory).resolve()
        run(root, setup_mail=configure)
