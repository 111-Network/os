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

Layer 5 artifacts (D20; os/50-messaging/README.md section 19 L1) are optional and taken under the
names their CI build scripts give them, so the tag stays in the published file name:
  meshmud-<tag>-linux-amd64 / -linux-arm64      scripts/meshmud-build.sh
      -> [pkg.meshmud.target.x86_64-unknown-linux-musl] / [...aarch64-unknown-linux-musl]
  meshtastic-web-<tag>.tar + meshtastic-web-<tag>.tree    scripts/meshtastic-web-build.sh
      -> [bundle.meshtastic-web], tree-sha256 = sha256 of the .tree file (the bundle hash)
  meshtastic-presence-lock.txt                  50-messaging/sidecars/meshtastic-presence/
      -> [presence-lock]
At most one file per pattern; a .tar without its .tree (or the reverse) is an error. The same
bytes ride inside the payloads (bin/meshmud, panels/meshtastic-web/, share/meshtastic-presence/).
Every node cross-checks its extracted payload against these pins (manifest.rs
`verify_layer5_tree`, called by the updater, the root helper and the installer), so a release
whose payloads disagree with its pins would be refused by every node. This generator runs the
same check first and refuses to write such a manifest:
  - the .tree must equal the listing recomputed from the .tar it sits beside;
  - a meshmud build needs the `111` binary and the payload-<triple>.tar.gz of its triple (the
    node refuses the whole manifest otherwise);
  - each payload-<triple>.tar.gz is read (never extracted) under the node's unpack rules (plain
    files and directories only, no absolute or `..` paths) and must carry exactly the pinned
    bin/meshmud, panels/meshtastic-web/ tree and presence lock, and nothing of the three that is
    not pinned (nothing else under panels/).
A payload that is not a readable .tar.gz is refused whenever any Layer 5 pin is written; with
none written it is reported on stderr and passed through (a node refuses it at unpack anyway).
"""
import argparse
import glob
import hashlib
import os
import sys
import tarfile

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


# Where the Layer 5 assets sit inside a payload (manifest.rs `layer5`).
PAYLOAD_MESHMUD = "bin/meshmud"
PAYLOAD_PANELS = "panels"
PAYLOAD_PRESENCE_LOCK = "share/meshtastic-presence/meshtastic-presence-lock.txt"
WEB_BUNDLE = "meshtastic-web"
# Bounds for reading a tar (a packaging error, not a release, beyond these).
TAR_MAX_MEMBERS = 262144


def fail(msg):
    sys.exit(f"gen-manifest: {msg}")


def member_path(name, what, allow_dot_prefix):
    """A tar member name as the node's safe_unpack sees it, '/'-joined; refuses what it refuses.

    Rust's Path::components() drops empty and interior '.' components but yields a leading '.'
    as CurDir, which safe_unpack refuses, so payload names must not start with './'. The bundle
    tar is written by `tar -C dir .`, so its names do (allow_dot_prefix)."""
    if name.startswith("/"):
        fail(f"{what}: absolute member path {name!r}")
    parts = name.split("/")
    out = []
    for i, part in enumerate(parts):
        if part == "":
            continue
        if part == ".":
            if i == 0 and not allow_dot_prefix:
                fail(f"{what}: member path {name!r} starts with './' (the node refuses it)")
            continue
        if part == "..":
            fail(f"{what}: member path {name!r} climbs out with '..'")
        out.append(part)
    return "/".join(out)


def read_tar(path, what, gz, allow_dot_prefix):
    """{relpath: ("file", sha256, size) | ("dir", None, None)} for every member, read in memory.

    Plain files and directories only (the node refuses links, hard links and devices); a path
    listed twice is refused. Raises tarfile.TarError/OSError/EOFError when unreadable."""
    entries = {}
    with tarfile.open(path, "r:gz" if gz else "r:") as tar:
        for count, m in enumerate(tar, 1):
            if count > TAR_MAX_MEMBERS:
                fail(f"{what}: more than {TAR_MAX_MEMBERS} members")
            rel = member_path(m.name, what, allow_dot_prefix)
            if m.type in (tarfile.REGTYPE, tarfile.AREGTYPE):
                h = hashlib.sha256()
                f = tar.extractfile(m)
                for chunk in iter(lambda: f.read(1 << 20), b""):
                    h.update(chunk)
                entry = ("file", h.hexdigest(), m.size)
            elif m.type == tarfile.DIRTYPE:
                entry = ("dir", None, None)
            else:
                fail(f"{what}: {m.name!r} is a link or special file (the node refuses it)")
            if rel == "":
                if entry[0] != "dir":
                    fail(f"{what}: a file member with an empty path")
                continue
            if rel in entries:
                if entry[0] == "dir" and entries[rel][0] == "dir":
                    continue
                fail(f"{what}: {rel} is listed twice")
            entries[rel] = entry
    return entries


def tree_listing(entries, prefix, what):
    """The bundle listing (manifest.rs `bundle_tree_sha256`; meshtastic-web-build.sh's .tree):
    `<sha256>  <relpath>` + newline for every file under prefix, relpaths sorted bytewise."""
    rows = []
    for rel, (kind, sha, _size) in entries.items():
        if kind != "file":
            continue
        if prefix:
            if not rel.startswith(prefix + "/"):
                continue
            rel = rel[len(prefix) + 1:]
        if any(c in rel for c in "\\\n\r"):
            fail(f"{what}: file name {rel!r} would be escaped by sha256sum")
        try:
            key = rel.encode("utf-8")
        except UnicodeEncodeError:
            fail(f"{what}: file name {rel!r} is not UTF-8")
        rows.append((key, f"{sha}  {rel}\n"))
    rows.sort(key=lambda r: r[0])
    return "".join(r[1] for r in rows).encode("utf-8")


def present(entries, rel):
    """Anything at rel, or anything beneath it."""
    return rel in entries or any(k.startswith(rel + "/") for k in entries)


def check_pinned(entries, rel, path, what):
    """The payload's rel is a plain file with the sha256 and size of the artifact at path."""
    got = entries.get(rel)
    if got is None or got[0] != "file":
        fail(f"{what}: carries no {rel} although the manifest pins {os.path.basename(path)}")
    want = (sha256_of(path), os.path.getsize(path))
    if (got[1], got[2]) != want:
        fail(f"{what}: {rel} (sha256 {got[1]}, {got[2]} bytes) differs from the pinned "
             f"{os.path.basename(path)} (sha256 {want[0]}, {want[1]} bytes)")


def check_payload(path, triple, meshmud, web_listing, lock, pins_any):
    """Cross-check one payload-<triple>.tar.gz against the Layer 5 pins about to be written
    (the node's verify_layer5_tree, both ways)."""
    what = os.path.basename(path)
    try:
        entries = read_tar(path, what, gz=True, allow_dot_prefix=False)
    except (tarfile.TarError, OSError, EOFError) as e:
        if pins_any:
            fail(f"{what}: not a readable .tar.gz ({e}); the Layer 5 pins cannot be checked against it")
        print(f"gen-manifest: warning: {what} is not a readable .tar.gz ({e}); no Layer 5 pin is "
              "written, so there is nothing to cross-check (a node refuses such a payload at unpack)",
              file=sys.stderr)
        return

    if meshmud is not None:
        check_pinned(entries, PAYLOAD_MESHMUD, meshmud, what)
    elif present(entries, PAYLOAD_MESHMUD):
        fail(f"{what}: carries {PAYLOAD_MESHMUD} but no meshmud build for {triple} is pinned")

    panels = entries.get(PAYLOAD_PANELS)
    if panels is not None and panels[0] != "dir":
        fail(f"{what}: {PAYLOAD_PANELS} is not a directory")
    web_dir = f"{PAYLOAD_PANELS}/{WEB_BUNDLE}"
    for rel, (kind, _sha, _size) in entries.items():
        if not rel.startswith(PAYLOAD_PANELS + "/"):
            continue
        top = rel.split("/")[1]
        if web_listing is None or top != WEB_BUNDLE:
            fail(f"{what}: carries {PAYLOAD_PANELS}/{top} but the manifest pins no such bundle")
        if rel == web_dir and kind != "dir":
            fail(f"{what}: {web_dir} is not a directory")
    if web_listing is not None:
        if not present(entries, web_dir):
            fail(f"{what}: lacks the pinned bundle {web_dir}/")
        got = tree_listing(entries, web_dir, what)
        if got != web_listing:
            fail(f"{what}: {web_dir}/ tree (sha256 {hashlib.sha256(got).hexdigest()}) differs from "
                 f"the pinned bundle's .tree (sha256 {hashlib.sha256(web_listing).hexdigest()})")

    if lock is not None:
        check_pinned(entries, PAYLOAD_PRESENCE_LOCK, lock, what)
    elif present(entries, PAYLOAD_PRESENCE_LOCK):
        fail(f"{what}: carries {PAYLOAD_PRESENCE_LOCK} but no presence lock is pinned")


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

    meshmud_for = {}
    for arch, triple in MESHMUD_ARCH.items():
        path = one_match(a.artifacts_dir, f"meshmud-*-linux-{arch}")
        if path is None:
            continue
        fname = os.path.basename(path)
        # The node refuses the whole manifest for a meshmud with no 111 binary or payload
        # of its triple (it rides in that payload), halting updates everywhere.
        for needed in (f"111-{triple}", f"payload-{triple}.tar.gz"):
            if not os.path.isfile(os.path.join(a.artifacts_dir, needed)):
                fail(f"{fname} is for {triple}, but {needed} is missing: meshmud rides in that "
                     "payload, and every node would refuse the whole manifest")
        meshmud_for[triple] = path
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
    web_listing = None
    if web_tar is not None:
        if web_tar[: -len(".tar")] != web_tree[: -len(".tree")]:
            sys.exit(f"meshtastic-web: {os.path.basename(web_tar)} and {os.path.basename(web_tree)} name different tags")
        fname = os.path.basename(web_tar)
        # The .tree must describe the .tar it sits beside (tree-sha256 is the .tree's hash).
        try:
            tar_entries = read_tar(web_tar, fname, gz=False, allow_dot_prefix=True)
        except (tarfile.TarError, OSError, EOFError) as e:
            fail(f"{fname}: not a readable tar ({e})")
        with open(web_tree, "rb") as f:
            web_listing = f.read()
        if tree_listing(tar_entries, "", fname) != web_listing:
            fail(f"{os.path.basename(web_tree)} does not match the listing of {fname} (stale or mixed build outputs)")
        lines += [
            "[bundle.meshtastic-web]",
            f'url = "{a.url_base}/{fname}"',
            f'sha256 = "{sha256_of(web_tar)}"',
            f"size = {os.path.getsize(web_tar)}",
            f'tree-sha256 = "{sha256_of(web_tree)}"',
            "",
        ]

    lock = os.path.join(a.artifacts_dir, "meshtastic-presence-lock.txt")
    lock = lock if os.path.isfile(lock) else None
    pins_any = bool(meshmud_for) or web_listing is not None or lock is not None
    for triple in MESHMUD_ARCH.values():
        payload = os.path.join(a.artifacts_dir, f"payload-{triple}.tar.gz")
        if os.path.isfile(payload):
            check_payload(payload, triple, meshmud_for.get(triple), web_listing, lock, pins_any)

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
