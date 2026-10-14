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

Layer 5 artifacts (D20; os/messaging/README.md section 19 L1) are optional and taken under the
names their CI build scripts give them, so the tag stays in the published file name:
  meshmud-<tag>-linux-amd64 / -linux-arm64      scripts/meshmud-build.sh
      -> [pkg.meshmud.target.x86_64-unknown-linux-musl] / [...aarch64-unknown-linux-musl]
  meshtastic-web-<tag>.tar + meshtastic-web-<tag>.tree    scripts/meshtastic-web-build.sh
      -> [bundle.meshtastic-web], tree-sha256 = sha256 of the .tree file (the bundle hash)
  meshtastic-presence-lock.txt                  messaging/sidecars/meshtastic-presence/
      -> [presence-lock]
At most one file per pattern; a .tar without its .tree (or the reverse) is an error. The same
bytes ride inside the payloads (bin/meshmud, panels/meshtastic-web/, share/meshtastic-presence/),
which the node cross-checks against these pins (manifest.rs `verify_layer5_tree`).
"""
import argparse
import glob
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
    "meshtastic-presence-lock.txt": ("presence-lock", "meshtastic-presence-lock.txt"),
}

# Layer 5 meshmud binaries: Go arch suffix of meshmud-build.sh -> target triple of the payload it rides in.
MESHMUD_ARCH = {
    "amd64": "x86_64-unknown-linux-musl",
    "arm64": "aarch64-unknown-linux-musl",
}


def one_match(artifacts_dir, pattern):
    """The single file matching pattern in artifacts_dir, or None; more than one is an error."""
    hits = sorted(glob.glob(os.path.join(glob.escape(artifacts_dir), pattern)))
    hits = [h for h in hits if os.path.isfile(h)]
    if len(hits) > 1:
        sys.exit(f"more than one artifact matches {pattern}: {', '.join(os.path.basename(h) for h in hits)}")
    return hits[0] if hits else None


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

    for arch, triple in MESHMUD_ARCH.items():
        path = one_match(a.artifacts_dir, f"meshmud-*-linux-{arch}")
        if path is None:
            continue
        fname = os.path.basename(path)
        lines += [
            f"[pkg.meshmud.target.{triple}]",
            f'url = "{a.url_base}/{fname}"',
            f'sha256 = "{sha256_of(path)}"',
            f"size = {os.path.getsize(path)}",
            "",
        ]

    web_tar = one_match(a.artifacts_dir, "meshtastic-web-*.tar")
    web_tree = one_match(a.artifacts_dir, "meshtastic-web-*.tree")
    if (web_tar is None) != (web_tree is None):
        sys.exit("meshtastic-web: the .tar and its .tree must both be present (or neither)")
    if web_tar is not None:
        if web_tar[: -len(".tar")] != web_tree[: -len(".tree")]:
            sys.exit(f"meshtastic-web: {os.path.basename(web_tar)} and {os.path.basename(web_tree)} name different tags")
        fname = os.path.basename(web_tar)
        lines += [
            "[bundle.meshtastic-web]",
            f'url = "{a.url_base}/{fname}"',
            f'sha256 = "{sha256_of(web_tar)}"',
            f"size = {os.path.getsize(web_tar)}",
            f'tree-sha256 = "{sha256_of(web_tree)}"',
            "",
        ]

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
