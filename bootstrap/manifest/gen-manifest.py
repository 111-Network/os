#!/usr/bin/env python3
"""gen-manifest — build channel-111-stable.toml from release artifacts.

Reads the artifacts in --artifacts-dir, computes sha256+size for each, and
emits a manifest validating against the T2 parser (round-trip checked by
crates/node111/tests/manifest_parse.rs::generated_manifest_parses).

Usage:
  gen-manifest.py --version 2026.10.12 --artifacts-dir dist/releases/2026.10.12 \
      --min-upgrade-from 2026.10.01 \
      --keychain-gen 1 --pubkey 'RW...' [--sig-by-prev '...'] \
      --url-base https://github.com/111-Network/os/releases/download/v2026.10.12 \
      --out dist/channel-111-stable.toml
"""
import argparse
import hashlib
import os
import sys

# Artifact layout in the artifacts dir (produced by scripts/cross-build.sh +
# the APK/deb prefetch steps of scripts/release.sh).
ARTIFACTS = {
    "111-x86_64-unknown-linux-musl": ("pkg.111.target.x86_64-unknown-linux-musl", "111"),
    "111-aarch64-unknown-linux-musl": ("pkg.111.target.aarch64-unknown-linux-musl", "111"),
    "payload-x86_64-unknown-linux-musl.tar.gz": ("payload.x86_64-unknown-linux-musl", "payload"),
    "payload-aarch64-unknown-linux-musl.tar.gz": ("payload.aarch64-unknown-linux-musl", "payload"),
    "termux-app.apk": ("apk.termux", "termux"),
    "termux-boot.apk": ("apk.termux-boot", "termux-boot"),
    "termux-api.apk": ("apk.termux-api", "termux-api"),
    "rns-lock.txt": ("rns-lock", "rns-lock.txt"),
    "kb.tar.gz": ("kb", "kb.tar.gz"),
}


def sha256_of(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True)
    ap.add_argument("--artifacts-dir", required=True)
    ap.add_argument("--min-upgrade-from", required=True)
    ap.add_argument("--url-base", required=True)
    ap.add_argument("--keychain-gen", type=int, default=0)
    ap.add_argument("--pubkey", required=True)
    ap.add_argument("--sig-by-prev", default=None)
    ap.add_argument("--debs-dir", default=None, help="directory of pinned .deb files (optional)")
    ap.add_argument("--out", required=True)
    a = ap.parse_args()

    lines = [
        "manifest-version = 1",
        f'date = "{a.version}"',
        'channel = "stable"',
        f'release = "{a.version}"',
        f'min-upgrade-from = "{a.min_upgrade_from}"',
        "",
    ]

    found_any = False
    for fname, (section, _label) in ARTIFACTS.items():
        path = os.path.join(a.artifacts_dir, fname)
        if not os.path.isfile(path):
            continue
        found_any = True
        sha = sha256_of(path)
        size = os.path.getsize(path)
        lines += [
            f"[{section}]",
            f'url = "{a.url_base}/{fname}"',
            f'sha256 = "{sha}"',
            f"size = {size}",
            "",
        ]
    if not found_any:
        sys.exit("no artifacts found in " + a.artifacts_dir)

    if a.debs_dir and os.path.isdir(a.debs_dir):
        for fname in sorted(os.listdir(a.debs_dir)):
            if not fname.endswith(".deb"):
                continue
            path = os.path.join(a.debs_dir, fname)
            name = fname.split("_")[0]
            lines += [
                "[[deb]]",
                f'name = "{name}"',
                f'url = "{a.url_base}/debs/{fname}"',
                f'sha256 = "{sha256_of(path)}"',
                f"size = {os.path.getsize(path)}",
                "",
            ]

    lines.append("[[keychain]]")
    lines.append(f"generation = {a.keychain_gen}")
    lines.append(f'pubkey = "{a.pubkey}"')
    if a.keychain_gen > 0:
        if not a.sig_by_prev:
            sys.exit("--sig-by-prev required for keychain generations > 0")
        lines.append(f'signature_by_previous = """{a.sig_by_prev}"""')
    lines.append("")

    with open(a.out, "w") as f:
        f.write("\n".join(lines))
    print(f"wrote {a.out}")


if __name__ == "__main__":
    main()
