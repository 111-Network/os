#!/usr/bin/env python3
"""verify/fixture_server.py — build a signed fixture channel (manifest+payload+
204+health) and serve it on 127.0.0.1:11180 for the namespace e2e.

Usage: fixture_server.py WORKDIR  → prints READY, serves until killed.
"""
import hashlib
import http.server
import io
import json
import os
import subprocess
import sys
import tarfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TEST_KEY = os.path.join(ROOT, "crates/node111/tests/fixtures/minisign/test.key")
RELEASE = "2026.10.12"
PORT = 11180


def sha(b):
    return hashlib.sha256(b).hexdigest()


def build(workdir):
    served = os.path.join(workdir, "served")
    os.makedirs(served, exist_ok=True)

    # payload tarball: bin/111 + share/kb/* + share/payload.manifest
    bin111 = open("/tmp/dist-test/111-x86_64-unknown-linux-musl", "rb").read()
    entries = {
        "bin/111": bin111,
        "share/kb/llms.txt": b"index\n",
        "share/kb/reticulum.md": b"entry\n",
    }
    manifest_lines = "".join(f"{sha(b)}  {p}\n" for p, b in entries.items())
    entries["share/payload.manifest"] = manifest_lines.encode()
    payload = os.path.join(served, "payload-x86_64-unknown-linux-musl.tar.gz")
    with tarfile.open(payload, "w:gz") as tar:
        for path, data in entries.items():
            info = tarfile.TarInfo(path)
            info.size = len(data)
            info.mode = 0o755 if path.startswith("bin/") else 0o644
            tar.addfile(info, io.BytesIO(data))

    # channel manifest via gen-manifest.py
    with open(os.path.join(served, "111-x86_64-unknown-linux-musl"), "wb") as f:
        f.write(bin111)
    with open(os.path.join(served, "rns-lock.txt"), "wb") as f:
        f.write(b"rns==1.5.6 --hash=sha256:" + b"ab" * 32 + b"\n")
    with open(os.path.join(served, "kb.tar.gz"), "wb") as f:
        f.write(b"fakekb")
    subprocess.run([
        "python3", os.path.join(ROOT, "manifest/gen-manifest.py"),
        "--version", RELEASE,
        "--artifacts-dir", served,
        "--min-upgrade-from", "2026.10.01",
        "--url-base", f"http://127.0.0.1:{PORT}",
        "--keychain-gen", "0",
        "--pubkey", "RWSwDusSn11Y2NID4fE2wwV6IZU8QwR7A5gtKtx1u6/ezzQ5WNzpjW4A",
        "--out", os.path.join(served, "channel-111-stable.toml"),
    ], check=True)
    subprocess.run([
        "minisign", "-S", "-W", "-s", TEST_KEY,
        "-m", os.path.join(served, "channel-111-stable.toml"),
        "-x", os.path.join(served, "channel-111-stable.toml.minisig"),
    ], check=True)

    # health + 204 endpoints (as files; the server maps them)
    with open(os.path.join(served, "health"), "w") as f:
        json.dump({"schema": 1, "status": "ok", "components": {
            "identity": True, "kb": True, "transport": "absent", "updater_idle": True}}, f)
    with open(os.path.join(served, "204"), "w") as f:
        f.write("")
    return served


class Handler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_GET(self):
        if self.path.strip("/") == "204":
            self.send_response(204)
            self.end_headers()
            return
        return super().do_GET()

    def do_HEAD(self):
        if self.path.strip("/") == "204":
            self.send_response(204)
            self.end_headers()
            return
        return super().do_HEAD()


def main():
    workdir = sys.argv[1]
    served = build(workdir)
    import functools
    import http.server
    server = http.server.ThreadingHTTPServer(
        ("127.0.0.1", PORT), functools.partial(Handler, directory=served)
    )
    print("READY", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
