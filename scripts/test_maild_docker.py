#!/usr/bin/env python3
"""Run real-client integration tests against a disposable fixture image."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time


def docker(*args, **kwargs):
    return subprocess.run(["docker", *args], check=True, text=True, **kwargs)


def main():
    image = sys.argv[1] if len(sys.argv) > 1 else "moonmail-test-maild:local"
    ports = {"imap_tls": 1993, "smtp_tls": 1465, "imap_starttls": 1143, "smtp_starttls": 1587}
    publish = [arg for port in ports.values() for arg in ("-p", f"127.0.0.1::{port}")]
    container = docker("run", "-d", "--read-only", "--tmpfs", "/state:uid=10001,gid=10001,mode=0700",
                       *publish, image, capture_output=True).stdout.strip()
    try:
        deadline = time.monotonic() + 30
        while True:
            status = json.loads(docker("inspect", container, capture_output=True).stdout)[0]
            if not status["State"]["Running"]:
                raise RuntimeError("fixture container exited before readiness")
            ready = subprocess.run(["docker", "exec", container, "test", "-f", "/state/ready.json"],
                                   capture_output=True)
            if ready.returncode == 0:
                break
            if time.monotonic() >= deadline:
                raise TimeoutError("fixture readiness timed out")
            time.sleep(0.1)
        with tempfile.TemporaryDirectory(prefix="moonmail-docker-") as directory:
            state = Path(directory)
            # docker cp does not expose tmpfs contents on every engine.
            ca = subprocess.run(["docker", "exec", container, "cat", "/state/ca.der"],
                                stdout=subprocess.PIPE, check=True).stdout
            (state / "ca.der").write_bytes(ca)
            bindings = status["NetworkSettings"]["Ports"]
            endpoints = {name: int(bindings[f"{port}/tcp"][0]["HostPort"]) for name, port in ports.items()}
            (state / "ready.json").write_text(json.dumps(endpoints))
            subprocess.run(["cargo", "test", "--locked", "-p", "moonmail-network", "--test", "local_mail",
                            "local_round_trip", "--", "--exact", "--nocapture"],
                           env={**os.environ, "MOONMAIL_TEST_MAILD_STATE": str(state)}, check=True)
    finally:
        subprocess.run(["docker", "logs", container], check=False)
        docker("rm", "-f", container, capture_output=True)


if __name__ == "__main__":
    main()
