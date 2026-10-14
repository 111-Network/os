#!/usr/bin/env python3
"""gen-notices — build os/THIRD-PARTY-NOTICES.md from what we actually ship.

Python 3 standard library only (written to run on 3.8+). Offline: every cargo
call uses --locked --offline, pnpm reads the installed node_modules.

  python3 -I scripts/gen-notices.py            regenerate os/THIRD-PARTY-NOTICES.md
  python3 -I scripts/gen-notices.py --out F    write to F instead (for diffing)
  python3 -I scripts/gen-notices.py --check    fast gate: compare the input SHA-256s
                                               (lockfiles, Go snapshots and license lists, the
                                               presence venv lock, the build-script pins) recorded
                                               in the notices file with the current inputs, and
                                               check the Go and Python license lists cover them;
                                               exit 1 when stale or incomplete

Inputs
  Rust   `cargo metadata` (repository, authors, crate dirs) and
         `cargo tree -e normal --target <musl triple>` (the shipped set) in
         os/bootstrap (binary `111`, includes id111) and os/intelligence
         (binary `111-agent`). os/identity is not an input: id111 is built
         inside the bootstrap workspace and resolved by bootstrap/Cargo.lock.
  JS     `pnpm licenses list --prod --json` in os/interface/ui.
  Go     (Layer 5) the meshmud closure: a go.mod / go.sum snapshot of the pinned upstream tag plus a
         hand-maintained license list, both under bootstrap/scripts/notices/. Go is a CI-only
         toolchain, so nothing here runs `go`; meshmud-build.sh checks the real binary against the
         same list. The pins (tag, commit, tarball sha256) are read from meshmud-build.sh.
  Python (Layer 5) the meshtastic-presence venv: the hash lock under
         messaging/sidecars/meshtastic-presence/ plus a hand-maintained license list
         (bootstrap/scripts/notices/python-licenses.txt). Every locked package needs a row.
  Web    (Layer 5) the Meshtastic web client bundle: tag, commit and hashes read from
         meshtastic-web-build.sh.
  Other  the four dashboard font files and their OFL texts, vendored Monocypher.

Output is sorted and has no timestamps: identical inputs give identical output.
"""
import argparse
import collections
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys

OS_DIR = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT_DEFAULT = os.path.join(OS_DIR, "THIRD-PARTY-NOTICES.md")

# (workspace dir, binary tag) — order does not matter, output is sorted
CARGO_WORKSPACES = (("bootstrap", "111"), ("intelligence", "111-agent"))
TARGETS = ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl")
UI_DIR = os.path.join(OS_DIR, "interface", "ui")
LOCKFILES = ("bootstrap/Cargo.lock", "intelligence/Cargo.lock", "interface/ui/pnpm-lock.yaml")
FONT_DIR = os.path.join(UI_DIR, "public", "fonts")
MONO_DIR = os.path.join(OS_DIR, "identity", "monocypher-harness")

# Layer 5 inputs (Go closure of meshmud, presence venv, web bundle pins)
SCRIPTS_DIR = os.path.join(OS_DIR, "bootstrap", "scripts")
NOTICES_DIR = os.path.join(SCRIPTS_DIR, "notices")
MESHMUD_SCRIPT = os.path.join(SCRIPTS_DIR, "meshmud-build.sh")
WEB_SCRIPT = os.path.join(SCRIPTS_DIR, "meshtastic-web-build.sh")
GO_LICENSES = os.path.join(NOTICES_DIR, "go-licenses.txt")
PY_LICENSES = os.path.join(NOTICES_DIR, "python-licenses.txt")
PRESENCE_DIR = os.path.join(OS_DIR, "messaging", "sidecars", "meshtastic-presence")
PRESENCE_LOCK = os.path.join(PRESENCE_DIR, "meshtastic-presence-lock.txt")
PRESENCE_LICENSE = os.path.join(PRESENCE_DIR, "LICENSE")


def die(msg):
    sys.stderr.write("gen-notices: " + msg + "\n")
    sys.exit(1)


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


# ------------------------------------------------------------------ Layer 5 inputs
def read_pins(path, prefix):
    """The `NAME="value"` pin lines of a build script (one assignment per line, double-quoted)."""
    if not os.path.isfile(path):
        die(f"{os.path.relpath(path, OS_DIR)} is missing")
    pins = {}
    pat = re.compile(r'^(' + prefix + r'_[A-Z0-9_]+)="([^"]*)"\s*$')
    for ln in read(path).splitlines():
        m = pat.match(ln)
        if m:
            pins[m.group(1)] = m.group(2)
    return pins


def need_pins(pins, names, where):
    for n in names:
        if not pins.get(n):
            die(f"{where}: pin {n} is missing or empty")


def pins_digest(pins):
    return hashlib.sha256("".join(f"{k}={pins[k]}\n" for k in sorted(pins)).encode("utf-8")).hexdigest()


def meshmud_pins():
    pins = read_pins(MESHMUD_SCRIPT, "MESHMUD")
    need_pins(pins, ("MESHMUD_REPO", "MESHMUD_TAG", "MESHMUD_COMMIT", "MESHMUD_TARBALL_SHA256"),
              "bootstrap/scripts/meshmud-build.sh")
    return pins


def web_pins():
    pins = read_pins(WEB_SCRIPT, "WEB")
    need_pins(pins, ("WEB_REPO", "WEB_TAG", "WEB_COMMIT"), "bootstrap/scripts/meshtastic-web-build.sh")
    return pins


def go_snapshot_paths(tag):
    return (os.path.join(NOTICES_DIR, f"meshmud-{tag}.go.mod"), os.path.join(NOTICES_DIR, f"meshmud-{tag}.go.sum"))


def input_records():
    """[(sha256, label)] for every input recorded in the notices file and compared by --check."""
    recs = []
    for rel in LOCKFILES:
        path = os.path.join(OS_DIR, rel)
        recs.append((sha256(path) if os.path.isfile(path) else None, rel))
    mp, wp = meshmud_pins(), web_pins()
    recs.append((pins_digest(mp), "bootstrap/scripts/meshmud-build.sh#pins"))
    recs.append((pins_digest(wp), "bootstrap/scripts/meshtastic-web-build.sh#pins"))
    gomod, gosum = go_snapshot_paths(mp["MESHMUD_TAG"])
    extra = (gomod, gosum, GO_LICENSES, PY_LICENSES, PRESENCE_LOCK, PRESENCE_LICENSE)
    for path in extra:
        recs.append((sha256(path) if os.path.isfile(path) else None, os.path.relpath(path, OS_DIR)))
    return recs


def read_rows(path, ncols):
    """Rows of a hand-maintained `a | b | c ...` list (comments with #, blank lines ignored)."""
    if not os.path.isfile(path):
        die(f"{os.path.relpath(path, OS_DIR)} is missing")
    rows = []
    for no, ln in enumerate(read(path).splitlines(), 1):
        t = ln.strip()
        if not t or t.startswith("#"):
            continue
        cols = [c.strip() for c in t.split("|")]
        if len(cols) != ncols or not all(cols):
            die(f"{os.path.relpath(path, OS_DIR)}:{no}: expected {ncols} non-empty columns separated by '|'")
        rows.append(tuple(cols))
    return rows


def parse_go_mod(text):
    """[(module, version, indirect)] from the `require` entries of a go.mod; replace/exclude are refused."""
    reqs, in_block = [], False
    for ln in text.splitlines():
        code = ln.split("//", 1)[0].strip()
        indirect = "// indirect" in ln
        if not code:
            continue
        if in_block:
            if code == ")":
                in_block = False
                continue
            parts = code.split()
            if len(parts) != 2:
                die(f"go.mod snapshot: cannot read the require entry {ln!r}")
            reqs.append((parts[0], parts[1], indirect))
        elif code.startswith("require ("):
            in_block = True
        elif code.startswith("require "):
            parts = code[len("require "):].split()
            if len(parts) != 2:
                die(f"go.mod snapshot: cannot read {ln!r}")
            reqs.append((parts[0], parts[1], indirect))
        elif code.startswith(("replace", "exclude", "retract")):
            die("go.mod snapshot has a replace/exclude/retract directive; the notices generator does not model those")
    if in_block:
        die("go.mod snapshot: unterminated require block")
    return reqs


def parse_go_sum(text):
    """{(module, version)} having a zip hash (`h1:`) line in go.sum."""
    out = set()
    for ln in text.splitlines():
        parts = ln.split()
        if len(parts) == 3 and parts[2].startswith("h1:") and not parts[1].endswith("/go.mod"):
            out.add((parts[0], parts[1]))
    return out


def parse_lock(text):
    """{canonical name: (version, marker-or-None)} from a pip --require-hashes lock."""
    pkgs = {}
    for ln in text.splitlines():
        m = re.match(r"^([A-Za-z0-9][A-Za-z0-9._-]*)==(\S+?)(?: ; (.+?))?(?: \\)?$", ln)
        if m:
            pkgs[re.sub(r"[-_.]+", "-", m.group(1)).lower()] = (m.group(2), m.group(3))
    return pkgs


KNOWN_EXTRA_LICENSES = {"GPL-3.0-only", "GPL-3.0-or-later", "MPL-2.0", "PSF-2.0"}


def collect_go(texts_ids=None):
    """Go closure rows of the pinned meshmud snapshot, cross-checked against go-licenses.txt."""
    pins = meshmud_pins()
    tag = pins["MESHMUD_TAG"]
    gomod_p, gosum_p = go_snapshot_paths(tag)
    for pth in (gomod_p, gosum_p):
        if not os.path.isfile(pth):
            die(f"{os.path.relpath(pth, OS_DIR)} is missing (snapshot of the pinned tag's go.mod / go.sum)")
    reqs = parse_go_mod(read(gomod_p))
    sums = parse_go_sum(read(gosum_p))
    if not reqs:
        die("the go.mod snapshot has no require entries")
    rows = {(r[0], r[1]): r for r in read_rows(GO_LICENSES, 5)}
    closure, problems = [], []
    for mod, ver, indirect in reqs:
        if (mod, ver) not in sums:
            problems.append(f"go.sum snapshot has no h1 hash for {mod} {ver}")
        row = rows.get((mod, ver))
        if row is None:
            problems.append(f"go.mod requires {mod} {ver}: no row with that version in notices/go-licenses.txt")
        else:
            closure.append(row)
    needed = {(m, v) for m, v, _ in reqs}
    main_mod = re.search(r"^module\s+(\S+)", read(gomod_p), re.M)
    main_row = rows.get((main_mod.group(1), tag)) if main_mod else None
    if main_row is None:
        problems.append(f"notices/go-licenses.txt has no row for the main module at {tag}")
    std = [r for (m, _), r in rows.items() if m == "std"]
    if len(std) != 1:
        problems.append("notices/go-licenses.txt needs exactly one `std` row (the Go standard library is linked in)")
    for key, row in rows.items():
        if key not in needed and row is not main_row and row not in std:
            problems.append(f"notices/go-licenses.txt lists {key[0]} {key[1]}, which the go.mod snapshot does not require")
    if problems:
        die("Go closure and license list disagree:\n  " + "\n  ".join(problems))
    closure.sort(key=lambda r: (r[2].lower(), r[0]))
    return {"pins": pins, "closure": closure, "main": main_row, "std": std[0], "n_sum": len(sums)}


def collect_python():
    """Presence venv rows: the lock's packages, cross-checked against python-licenses.txt."""
    if not os.path.isfile(PRESENCE_LOCK):
        die(f"{os.path.relpath(PRESENCE_LOCK, OS_DIR)} is missing")
    lock = parse_lock(read(PRESENCE_LOCK))
    if not lock:
        die("the presence lock has no pinned packages")
    rows = {}
    for r in read_rows(PY_LICENSES, 5):
        key = re.sub(r"[-_.]+", "-", r[0]).lower()
        if key in rows:
            die(f"notices/python-licenses.txt lists {r[0]} twice")
        rows[key] = r
    problems = []
    for name, (ver, _marker) in sorted(lock.items()):
        r = rows.get(name)
        if r is None:
            problems.append(f"the lock pins {name}=={ver}: no row in notices/python-licenses.txt")
        elif r[1] != ver:
            problems.append(f"the lock pins {name}=={ver}, notices/python-licenses.txt says {r[1]}")
    for name in rows:
        if name not in lock:
            problems.append(f"notices/python-licenses.txt lists {name}, which is not in the lock")
    if problems:
        die("presence venv lock and license list disagree:\n  " + "\n  ".join(problems))
    out = [rows[n] for n in sorted(lock, key=lambda n: (rows[n][2].lower(), n))]
    return {"rows": out, "lock": lock}


def check_license_ids(rows, label, known):
    """Every SPDX id used by a hand list must have a text (or a stated pointer) in the output."""
    for r in rows:
        for i in re.findall(r"[A-Za-z0-9][A-Za-z0-9.\-+]*", re.sub(r"\b(OR|AND|WITH)\b", " ", r[2])):
            if i not in known:
                die(f"{label}: {r[0]} uses license id {i}, which has no text in build_texts(); add the text first")


# ------------------------------------------------------------------ --check
RECORD = re.compile(r"^([0-9a-f]{64})  (\S+)$", re.M)


def check(out_path):
    if not os.path.isfile(out_path):
        die(f"{out_path} is missing — run: python3 -I os/bootstrap/scripts/gen-notices.py")
    text = read(out_path)
    marker = text.find("Inputs at generation time")
    if marker < 0:
        die("no 'Inputs at generation time' block in the notices file — run: python3 -I os/bootstrap/scripts/gen-notices.py")
    block = text[marker:text.find("```", text.find("```", marker) + 3)]
    recorded = {rel: h for h, rel in RECORD.findall(block)}
    # the coverage checks first: they name the missing row, which is more useful than "changed"
    collect_go()
    collect_python()
    stale = []
    for h, rel in input_records():
        if rel not in recorded:
            stale.append(f"  {rel}: not recorded in THIRD-PARTY-NOTICES.md")
        elif h is None:
            stale.append(f"  {rel}: recorded but the file is gone")
        elif h != recorded[rel]:
            stale.append(f"  {rel}: changed (recorded {recorded[rel][:12]}…, now {h[:12]}…)")
    if stale:
        sys.stderr.write("THIRD-PARTY-NOTICES.md is stale — an input changed since it was generated:\n")
        sys.stderr.write("\n".join(stale) + "\n")
        sys.stderr.write("Run: python3 -I os/bootstrap/scripts/gen-notices.py   (then review and commit the result)\n")
        sys.exit(1)
    print("THIRD-PARTY-NOTICES.md is current (input hashes match; Go and Python license lists cover their locks)")


# ------------------------------------------------------------------ helpers
def tool_env():
    env = dict(os.environ)
    extra = [os.path.expanduser("~/.cargo/bin"), os.path.expanduser("~/.local/bin")]
    env["PATH"] = os.pathsep.join(extra + [env.get("PATH", "")])
    return env


def run(cmd, cwd):
    exe = shutil.which(cmd[0], path=tool_env()["PATH"])
    if not exe:
        die(f"'{cmd[0]}' not found on PATH (cargo lives in ~/.cargo/bin)")
    r = subprocess.run([exe] + cmd[1:], cwd=cwd, env=tool_env(), stdout=subprocess.PIPE,
                       stderr=subprocess.PIPE, universal_newlines=True)
    if r.returncode != 0:
        die(f"{' '.join(cmd)} (in {cwd}) failed:\n{r.stderr.strip()}")
    return r.stdout


LICFILE = re.compile(r"^(LICEN[CS]E|COPYING|UNLICENSE|COPYRIGHT|NOTICE)", re.I)
BAD = ("yyyy", "[name", "<year>", "notice", "statement", "owner", "holder", "{")


def copyright_lines(path, skip=()):
    out = []
    for f in sorted(os.listdir(path)):
        full = os.path.join(path, f)
        if f in skip or not LICFILE.match(f) or not os.path.isfile(full):
            continue
        try:
            txt = read(full)
        except (OSError, UnicodeDecodeError):
            continue
        for ln in txt.splitlines():
            t = ln.strip()
            if t.startswith("Copyright") and not any(b in t.lower() for b in BAD):
                t = re.sub(r"\s+", " ", t).rstrip(".")
                t = t.replace(" All rights reserved", "")
                t = re.sub(r"\s*<[^>]*@[^>]*>", "", t)  # drop e-mail addresses
                if t not in out:
                    out.append(t)
    return out


def clean_authors(a):
    return [re.sub(r"\s*<[^>]*>", "", x).strip() for x in (a or []) if x.strip()]


def norm_expr(e):
    e = re.sub(r"\s*/\s*", " OR ", e).strip()
    if "(" in e or " AND " in e:
        return e
    return " OR ".join(sorted(x.strip() for x in e.split(" OR ")))


# ------------------------------------------------------------------ collect: Rust
def collect_rust():
    meta, own, crates = {}, set(), {}
    for ws, tag in CARGO_WORKSPACES:
        wdir = os.path.join(OS_DIR, ws)
        m = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked", "--offline"], wdir))
        members = set(m["workspace_members"])
        for p in m["packages"]:
            meta[(p["name"], p["version"])] = p
            if p["id"] in members or p.get("source") is None:
                own.add(p["name"])  # workspace members and path crates (id111) are ours
        for t in TARGETS:
            out = run(["cargo", "tree", "--locked", "--offline", "-e", "normal", "--target", t,
                       "--prefix", "none", "--no-dedupe", "-f", "{p}|{l}"], wdir)
            for line in out.splitlines():
                if "|" not in line:
                    continue
                pv, lic = line.split("|", 1)
                name, ver = pv.split(" v", 1)
                ver = ver.split(" ")[0]
                d = crates.setdefault((name, ver), {"lic": lic, "bins": set()})
                d["bins"].add(tag)
    rows = []
    for (name, ver), d in crates.items():
        if name in own:
            continue
        p = meta[(name, ver)]
        repo = p.get("repository") or p.get("homepage") or f"https://crates.io/crates/{name}"
        cdir = os.path.dirname(p["manifest_path"])
        cr = copyright_lines(cdir, skip=("LICENSE-BoringSSL",) if name == "ring" else ())
        if cr:
            cp = "; ".join(cr[:3]) + (" ..." if len(cr) > 3 else "")
        else:
            au = clean_authors(p.get("authors"))
            cp = (("authors: " + ", ".join(au[:3]) + (" ..." if len(au) > 3 else "")) if au
                  else "no copyright line in package; see repository")
        bins = "both" if len(d["bins"]) == 2 else sorted(d["bins"])[0]
        rows.append((norm_expr(d["lic"]), name, ver, repo, cp, bins))
    rows.sort(key=lambda r: (r[0].lower(), r[1], r[2]))
    dirs = {}
    for (name, ver), p in meta.items():
        if (name, ver) in crates and name not in own:
            dirs.setdefault(name, (ver, os.path.dirname(p["manifest_path"])))
    return rows, dirs


# ------------------------------------------------------------------ collect: JS
def collect_js():
    data = json.loads(run(["pnpm", "licenses", "list", "--prod", "--json"], UI_DIR))
    rows = []
    for lic, pk in data.items():
        for p in pk:
            for ver, path in zip(p["versions"], p["paths"]):
                lic_files = [f for f in sorted(os.listdir(path)) if LICFILE.match(f)] if os.path.isdir(path) else []
                cr = copyright_lines(path) if os.path.isdir(path) else []
                pj = json.loads(read(os.path.join(path, "package.json")))
                repo = pj.get("repository")
                repo = repo.get("url") if isinstance(repo, dict) else repo
                repo = repo or pj.get("homepage") or f"https://www.npmjs.com/package/{p['name']}"
                repo = re.sub(r"^git\+", "", repo)
                if re.fullmatch(r"[\w.-]+/[\w.-]+", repo):
                    repo = "https://github.com/" + repo
                cp = ("; ".join(cr[:3]) if cr else
                      ("none in package; license from package.json" if not lic_files else "no copyright line found"))
                rows.append((lic, p["name"], ver, repo, cp))
    rows.sort(key=lambda r: (r[0].lower(), r[1], r[2]))
    return rows


# ------------------------------------------------------------------ license texts
def build_texts(dirs):
    def crate_file(crate, fname):
        if crate not in dirs:
            die(f"license text source crate '{crate}' is no longer in the shipped set; "
                "pick another local copy in build_texts()")
        ver, d = dirs[crate]
        return ver, read(os.path.join(d, fname))

    v, src = crate_file("byteorder", "LICENSE-MIT")
    mit_ver = v
    mit = ("MIT License\n\nCopyright (c) <year> <copyright holders: see the per-component lines above>\n\n"
           + src[src.index("Permission is hereby granted"):].strip())

    v, src = crate_file("ring", "LICENSE-BoringSSL")
    ring_ver = v
    apache = src[:src.index("limitations under the License.") + len("limitations under the License.")].strip("\n")

    v, src = crate_file("curve25519-dalek", "LICENSE")
    dalek_ver = v
    bsd3 = ("Copyright (c) <year> <copyright holders: see the per-component lines above>\n\n"
            + src[src.index("Redistribution and use"):].strip())

    mono = read(os.path.join(MONO_DIR, "vendor", "LICENCE.md"))
    bsd2 = ("Copyright (c) <year> <copyright holders: see the per-component lines above>\nAll rights reserved.\n\n"
            + mono[mono.index("Redistribution and use"):mono.index("Licence 2 (CC-0)")].strip())

    _, src = crate_file("ring", "LICENSE-other-bits")
    isc = ("Copyright (c) <year> <copyright holders: see the per-component lines above>\n\n"
           + src[src.index("Permission to use"):].strip())

    bip_ver, cc0 = crate_file("bip39", "LICENSE")
    cc0 = cc0.strip("\n")
    _, unlicense = crate_file("byteorder", "UNLICENSE")
    unlicense = unlicense.strip("\n")
    uni_ver, unicode = crate_file("unicode-ident", "LICENSE-UNICODE")
    unicode = unicode.strip("\n")
    ofl = read(os.path.join(FONT_DIR, "LICENSE-geist.txt")).strip("\n")
    cdla_ver, cdla = crate_file("webpki-roots", "LICENSE")
    cdla = cdla.strip("\n")
    if not os.path.isfile(PRESENCE_LICENSE):
        die("messaging/sidecars/meshtastic-presence/LICENSE (the GPL-3.0 text) is missing")
    gpl = read(PRESENCE_LICENSE).strip("\n")
    if "GNU GENERAL PUBLIC LICENSE" not in gpl or "Version 3, 29 June 2007" not in gpl:
        die("messaging/sidecars/meshtastic-presence/LICENSE is not the GPL-3.0 text")

    return [
        ("MIT", "https://spdx.org/licenses/MIT.html", f"template body from a local copy (`byteorder` {mit_ver} `LICENSE-MIT`); the copyright line is generic and each component's own line is listed above", mit),
        ("Apache-2.0", "https://www.apache.org/licenses/LICENSE-2.0", f"full text from a local copy (`ring` {ring_ver} `LICENSE-BoringSSL`, which carries the unmodified Apache License 2.0)", apache),
        ("BSD-3-Clause", "https://spdx.org/licenses/BSD-3-Clause.html", f"body from a local copy (`curve25519-dalek` {dalek_ver} `LICENSE`); generic copyright line", bsd3),
        ("BSD-2-Clause", "https://spdx.org/licenses/BSD-2-Clause.html", "body from the vendored Monocypher `LICENCE.md`; generic copyright line (Monocypher's own lines are in its entry)", bsd2),
        ("ISC", "https://spdx.org/licenses/ISC.html", f"body from a local copy (`ring` {ring_ver} `LICENSE-other-bits`); generic copyright line", isc),
        ("CC0-1.0", "https://creativecommons.org/publicdomain/zero/1.0/legalcode", f"full text from a local copy (`bip39` {bip_ver} `LICENSE`)", cc0),
        ("Unlicense", "https://unlicense.org/", f"full text from a local copy (`byteorder` {mit_ver} `UNLICENSE`)", unlicense),
        ("Unicode-3.0", "https://www.unicode.org/license.txt", f"full text from a local copy (`unicode-ident` {uni_ver} `LICENSE-UNICODE`)", unicode),
        ("OFL-1.1", "https://openfontlicense.org/open-font-license-official-text/", "full text as shipped with Geist (`interface/ui/public/fonts/LICENSE-geist.txt`). The Silkscreen file differs only in its copyright line and trivial typography, shown under Fonts", ofl),
        ("CDLA-Permissive-2.0", "https://cdla.dev/permissive-2-0/", f"full text from a local copy (`webpki-roots` {cdla_ver} `LICENSE`). Applies to the embedded Mozilla CA root data", cdla),
        ("GPL-3.0", "https://www.gnu.org/licenses/gpl-3.0.txt", "full text, unmodified, from `messaging/sidecars/meshtastic-presence/LICENSE` (the same text as the LICENSE of the pinned Meshtastic web client). Applies to the GPL-3.0-only and GPL-3.0-or-later components under Source offers", gpl),
    ]


def monocypher_entry():
    vendor = read(os.path.join(MONO_DIR, "VENDOR.md"))
    m = re.search(r"\|\s*Version\s*\|\s*\*\*([\d.]+)\*\*", vendor)
    if not m:
        die("cannot read the Monocypher version from identity/monocypher-harness/VENDOR.md")
    lic = read(os.path.join(MONO_DIR, "vendor", "LICENCE.md"))
    cps = [re.sub(r"^(Copyright \(c\) [\d-]+),", r"\1", ln.strip())
           for ln in lic.splitlines() if ln.startswith("Copyright (c)")]
    if not cps:
        die("no copyright lines found in the vendored Monocypher LICENCE.md")
    return m.group(1), "; ".join(cps) + "."


def font_rows():
    geist = read(os.path.join(FONT_DIR, "LICENSE-geist.txt")).splitlines()[0].strip()
    silk = read(os.path.join(FONT_DIR, "LICENSE-silkscreen.txt")).splitlines()[0].strip()
    g_src = "https://github.com/vercel/geist-font (npm package `geist` 1.7.0)"
    s_src = "https://github.com/googlefonts/silkscreen (served by Google Fonts)"
    return [
        ("Geist Mono", "GeistMono-Variable.woff2", geist, g_src),
        ("Geist Pixel Line", "GeistPixel-Line.woff2", geist, g_src),
        ("Silkscreen 400", "Silkscreen-400-latin.woff2", silk, s_src),
        ("Silkscreen 700", "Silkscreen-700-latin.woff2", silk, s_src),
    ], silk


# ------------------------------------------------------------------ render
def render():
    rust_rows, dirs = collect_rust()
    js_rows = collect_js()
    texts = build_texts(dirs)
    fonts, silk_line = font_rows()
    mono_ver, mono_cp = monocypher_entry()
    ver_of = {name: ver for name, (ver, _) in dirs.items()}
    go = collect_go()
    py = collect_python()
    web = web_pins()
    known_ids = {t[0] for t in texts} | KNOWN_EXTRA_LICENSES
    check_license_ids(go["closure"] + [go["main"], go["std"]], "notices/go-licenses.txt", known_ids)
    check_license_ids(py["rows"], "notices/python-licenses.txt", known_ids)

    ids = set()
    for r in rust_rows:
        ids |= set(re.findall(r"[A-Za-z0-9][A-Za-z0-9.\-+]*", re.sub(r"\b(OR|AND|WITH)\b", " ", r[0])))
    reproduced = {t[0] for t in texts} | {"LLVM-exception"}
    not_elected = sorted(i for i in ids if i not in reproduced)

    L = []
    w = L.append
    w("# Third-party notices")
    w("")
    w("The 111network OS includes the third-party components listed below. Each keeps its own license; the licenses are reproduced here")
    w("so that their notice and attribution requirements are met. 111network's own license is in `LICENSE.md` (draft). The reasoning behind")
    w("this list is in `LICENSING.md`.")
    w("")
    w("**Release requirements (follow-ups for the lead, not done by this file):**")
    w("")
    w("1. This file must ship inside the release payload, as a Layer 1 manifest item, so every installed node carries it.")
    w("2. It must be reachable from the dashboard: Settings, Essentials, a \"Licenses\" link.")
    w("")
    w("## Scope and how this file is generated")
    w("")
    w("- **Rust:** all normal dependencies of `111` (crate `node111`, which includes `id111`) and `111-agent` (crate `agent111`) for the Linux musl")
    w("  targets `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`, excluding our own crates. It includes compile-time proc-macro crates.")
    w("  Source: `cargo metadata` and `cargo tree -e normal --target <triple>` in `os/bootstrap` and `os/intelligence`.")
    w("- **JavaScript:** the production dependencies of the dashboard, from `pnpm licenses list --prod --json` in `os/interface/ui`.")
    w("- **Go (Layer 5):** the module closure of `meshmud`, the Meshtastic multiplexer, built in CI from a pinned upstream tarball into static")
    w("  binaries. Source: a snapshot of the tag's `go.mod` / `go.sum` and the hand-maintained `bootstrap/scripts/notices/go-licenses.txt`; the CI build")
    w("  fails when the real binary links a module that list does not cover. Go is a CI-only toolchain, so this generator never runs `go`.")
    w("- **Python (Layer 5):** the packages locked for the `meshtastic-presence` venv (`messaging/sidecars/meshtastic-presence/meshtastic-presence-lock.txt`),")
    w("  with the hand-maintained `bootstrap/scripts/notices/python-licenses.txt`. Every locked package needs a row at its locked version.")
    w("- **Web client bundle (Layer 5):** the Meshtastic web client, tag and commit from `bootstrap/scripts/meshtastic-web-build.sh`.")
    w("- **Fonts, vendored code, embedded data:** see their sections.")
    w("- **Copyright lines** are taken from each component's license files (Rust: the unpacked crate in the cargo registry; JS: the installed package). Where")
    w("  a component's license files have no copyright line, its `authors` metadata is shown instead.")
    w("- **Choice of license.** Where a component offers a choice (`OR`), the text of any one offered license suffices. This file reproduces the MIT or Apache-2.0")
    w("  option wherever one is offered.")
    w("- Not covered here: separate programs that run next to our code and are not linked (Reticulum, `meshtasticd` and the Meshtastic firmware, Termux")
    w("  and its packages, llama.cpp, opencode, RNode firmware). See `LICENSING.md` section 3. Their texts and source offers are added when a release carries them.")
    w("")
    w("Inputs at generation time (SHA-256):")
    w("")
    w("```")
    for h, rel in input_records():
        if h is None:
            die(f"input {rel} is missing")
        w(f"{h}  {rel}")
    w("```")
    w("")
    w("Regenerate with `python3 -I os/bootstrap/scripts/gen-notices.py` whenever an input above changes (a `#pins` entry is a hash of the pin lines of that build script). `--check` (run by `os/bootstrap/scripts/check.sh`) fails when the hashes above no longer match or a Go or Python license list no longer covers its lock.")
    w("The output is sorted and contains no timestamps, so identical inputs give identical output.")
    w("")

    cnt = collections.Counter(r[0] for r in rust_rows)
    w("## Summary")
    w("")
    w("| Group | Components |")
    w("|---|---|")
    w(f"| Rust crates | {len(rust_rows)} |")
    w(f"| JavaScript packages | {len(js_rows)} |")
    w(f"| Go modules (meshmud closure, CI-built binary) | {len(go['closure'])} + meshmud + the Go standard library |")
    w(f"| Python packages (presence venv) | {len(py['rows'])} |")
    w("| Web client bundle | 1 (Meshtastic web client, GPL-3.0-only, served unmodified) |")
    w(f"| Fonts | {len(fonts)} files, 3 families |")
    w("| Vendored source | 1 (Monocypher) |")
    w("| Embedded data | 1 (Mozilla CA roots, via `webpki-roots`, also counted under Rust crates) |")
    w("")

    w("## Rust crates")
    w("")
    w(f"{len(rust_rows)} crates, grouped by the license expression in their `Cargo.toml`. The last field says which binary links the crate:")
    w("`111`, `111-agent` or `both`.")
    w("")
    cur = None
    for lic, name, ver, repo, cp, bins in rust_rows:
        if lic != cur:
            cur = lic
            w("")
            w(f"### {lic} ({cnt[lic]})")
            w("")
        w(f"- `{name}` {ver} — {repo} — {cp} — {bins}")
    w("")
    w("Notes:")
    w("")
    if "ring" in ver_of:
        w(f"- `ring` {ver_of['ring']} (Apache-2.0 AND ISC): its own code is ISC (Copyright 2015-2025 Brian Smith). Code taken from BoringSSL is under Apache-2.0 or ISC as stated at the top of each file.")
        w("  Its `LICENSE-BoringSSL` also lists licenses for test-suite code that is not compiled into the library.")
    if "matchit" in ver_of:
        w(f"- `matchit` {ver_of['matchit']} (MIT AND BSD-3-Clause): the BSD-3-Clause part is code from `julienschmidt/httprouter` (its `LICENSE.httprouter`).")
    if "unicode-ident" in ver_of:
        w(f"- `unicode-ident` {ver_of['unicode-ident']} ((MIT OR Apache-2.0) AND Unicode-3.0): the Unicode-3.0 part covers data files from Unicode, Inc.")
    if "webpki-roots" in ver_of:
        w(f"- `webpki-roots` {ver_of['webpki-roots']} (CDLA-Permissive-2.0): Mozilla's CA root certificates, embedded as data in the `111` binary for TLS. The agreement text is reproduced below, as its section 2.1 requires.")
    if "libsqlite3-sys" in ver_of:
        sc_dir = os.path.join(dirs["libsqlite3-sys"][1], "sqlcipher")
        sc_cp = copyright_lines(sc_dir) if os.path.isdir(sc_dir) else []
        w(f"- `libsqlite3-sys` {ver_of['libsqlite3-sys']} (MIT): compiles native sources bundled in the crate. With the `bundled-sqlcipher` features that is SQLCipher"
          + (f" ({'; '.join(sc_cp[:2])}; BSD-3-Clause style license in the crate's `sqlcipher/LICENSE`)" if sc_cp else " (BSD-3-Clause style license in the crate's `sqlcipher/LICENSE`)")
          + ", built on SQLite (public domain). The BSD-3-Clause text below covers the SQLCipher notice.")
    # openssl-src is a build-dependency (absent from `cargo tree -e normal`), yet the OpenSSL it
    # compiles is linked into the static `111` binary as SQLCipher's crypto provider, so it is found
    # in the lockfile instead of the normal-dependency closure.
    ossl = re.search(r'name = "openssl-src"\nversion = "([^"]+)"', read(os.path.join(OS_DIR, "bootstrap", "Cargo.lock")))
    if ossl:
        ossl_crate = ossl.group(1)
        ossl_upstream = ossl_crate.split("+", 1)[1] if "+" in ossl_crate else ossl_crate
        w(f"- `openssl-src` {ossl_crate} (MIT OR Apache-2.0; a build-dependency, so not in the crate list above): bundles the OpenSSL {ossl_upstream} sources and compiles them into the static `111` binary as SQLCipher's crypto provider (`bundled-sqlcipher-vendored-openssl`); no system libcrypto is linked. OpenSSL {ossl_upstream} is Apache-2.0 (Copyright The OpenSSL Project Authors; see https://github.com/openssl/openssl/blob/master/LICENSE.txt); the Apache License 2.0 text below covers it.")
    w("- Options named in expressions but not elected, so not reproduced: " + ", ".join(not_elected) + ".")
    w("")

    w("## JavaScript packages (dashboard bundle)")
    w("")
    w(f"{len(js_rows)} production packages from `interface/ui`.")
    w("")
    for lic, name, ver, repo, cp in js_rows:
        w(f"- `{name}` {ver} — {lic} — {repo} — {cp}")
    w("")
    w("`wouter` publishes no license file in its package; its license is the `Unlicense` declared in `package.json`. Dev-only tooling is not shipped and is not listed.")
    w("")

    w("## Go modules (meshmud, CI-built)")
    w("")
    mp = go["pins"]
    w(f"`meshmud` {mp['MESHMUD_TAG']} ({mp['MESHMUD_REPO']}), the Meshtastic multiplexer, is built in CI from the pinned upstream tarball (sha256 `{mp['MESHMUD_TARBALL_SHA256']}`,")
    w(f"commit `{mp['MESHMUD_COMMIT']}`) into static linux/amd64 and linux/arm64 binaries and runs as a separate supervised process. Its source is never copied into this repository.")
    w(f"The closure is the `require` list of the tag's go.mod ({len(go['closure'])} modules; snapshots in `bootstrap/scripts/notices/`); `meshmud-build.sh` fails the CI build when the built binary")
    w("links a module that is not listed here.")
    w("")
    mm = go["main"]
    w(f"- **meshmud** {mm[1]} — {mm[2]} — {mm[4]} — {mm[3]} (the main module; its own code is MIT)")
    sd = go["std"]
    w(f"- **Go standard library and runtime** — {sd[2]} — {sd[4]} — {sd[3]} (linked statically into every Go binary; the toolchain version is recorded in the build's `.buildinfo`)")
    for mod, ver, lic, cp, repo in go["closure"]:
        w(f"- `{mod}` {ver} — {lic} — {repo} — {cp}")
    w("")
    mesh_paths = [m for m, _v, _lic, _cp, _r in go["closure"] if "meshtastic" in m.lower()]
    if mesh_paths:
        w("!! The go.mod snapshot lists a Meshtastic module (" + ", ".join(mesh_paths) + "): the binary links GPL code and must be conveyed under GPL-3.0.")
    else:
        w("The go.mod snapshot lists no Meshtastic module, so no Meshtastic Go protobufs are linked at this pin (the source decodes the few frame fields it needs with its own wire parser).")
        w("`meshmud-build.sh` repeats the check on the built binary and writes a verdict file. If a future pin ever links them, the binary is conveyed under GPL-3.0 and the source offer below applies.")
    w("")

    w("## Python packages (meshtastic-presence venv)")
    w("")
    w(f"{len(py['rows'])} packages locked in `messaging/sidecars/meshtastic-presence/meshtastic-presence-lock.txt` (every published file of each version hashed). The venv is built on the node by")
    w("`pip install --require-hashes`; we ship the lock and the reader's own source, not these packages. They are listed so the GPL source offer and the licenses are in one place.")
    w("")
    for name, ver, lic, cp, repo in py["rows"]:
        marker = py["lock"][re.sub(r"[-_.]+", "-", name).lower()][1]
        w(f"- `{name}` {ver} — {lic} — {repo} — {cp}" + (f" — only when `{marker}`" if marker else ""))
    w("")
    w("`certifi` (MPL-2.0) and `typing-extensions` (PSF-2.0) are installed unmodified; their full license texts are in each package's `*.dist-info/licenses/` on the node and are not copied here.")
    w("Canonical texts: https://www.mozilla.org/MPL/2.0/ and https://docs.python.org/3/license.html")
    w("")

    w("## Meshtastic web client bundle")
    w("")
    w(f"- **Meshtastic web client** {web['WEB_TAG']} — GPL-3.0-only — {web['WEB_REPO']}/tree/{web['WEB_TAG']} (commit `{web['WEB_COMMIT']}`)")
    w("  Hosted by the dashboard as a panel from its own directory, **served unmodified** and never embedded in the `111` binary. The bundle comes from the upstream release asset when it is complete,")
    w("  otherwise from a build of the pinned tag (`bootstrap/scripts/meshtastic-web-build.sh`); both are the same bytes where they overlap." +
      (f" Release asset `build.tar` sha256 `{web['WEB_ASSET_SHA256']}`." if web.get("WEB_ASSET_SHA256") else " Upstream ships no release asset for this tag."))
    if web.get("WEB_TREE_SHA256"):
        w(f"  Pinned bundle tree hash (`sha256` of the sorted `sha256  path` list): `{web['WEB_TREE_SHA256']}`.")
    w("  The bundle contains minified third-party JavaScript (React, Radix UI, maplibre-gl, sqlite-wasm and others, mostly MIT/ISC/BSD/Apache-2.0) that keeps its own licenses;")
    w("  `meshtastic-web-build.sh --licenses FILE` writes the `pnpm licenses` list of the tagged workspace for review (open item in `LICENSING.md` section 9).")
    w("")

    w("## Fonts")
    w("")
    w("Self-hosted woff2 files in the dashboard, unmodified copies (provenance in `interface/FONTS.md`). All four are under the SIL Open Font License 1.1.")
    w("")
    w("| Font | File | SHA-256 | Copyright | Source |")
    w("|---|---|---|---|---|")
    for fam, fn, cpy, src in fonts:
        w(f"| {fam} | `{fn}` | `{sha256(os.path.join(FONT_DIR, fn))}` | {cpy} | {src} |")
    w("")
    w("The OFL text reproduced below is Geist's. Silkscreen's `LICENSE-silkscreen.txt` is the same license with the Silkscreen copyright line above and minor typographic differences.")
    w("The fonts may be bundled with software under any license but may not be sold on their own.")
    w("")

    w("## Vendored source")
    w("")
    w(f"- **Monocypher** {mono_ver} — `BSD-2-Clause OR CC0-1.0` — https://monocypher.org — {mono_cp}")
    w("  Files: `identity/monocypher-harness/vendor/monocypher.{c,h}` and `optional/monocypher-ed25519.{c,h}`, hash-pinned in `SHA256SUMS`. It is used only by a host-side test harness and is not linked into any")
    w("  shipped binary. Its full dual license is `identity/monocypher-harness/vendor/LICENCE.md`; the BSD-2-Clause and CC0-1.0 texts are reproduced below.")
    w("")

    w("## Source offers (GPL-3.0 components)")
    w("")
    w("The GPL-3.0 programs below are conveyed or installed next to our code as separate programs; none is linked into the `111` or `111-agent` binaries. The corresponding source of each")
    w("is the pinned upstream source named here, plus our own reader source in this repository. The GPL-3.0 text is reproduced under License texts. (A lawyer's read of the offer")
    w("form is an open item, `LICENSING.md` section 9.)")
    w("")
    w(f"- **Meshtastic web client** {web['WEB_TAG']}: {web['WEB_REPO']}/tree/{web['WEB_TAG']} (commit `{web['WEB_COMMIT']}`). Build recipe: `bootstrap/scripts/meshtastic-web-build.sh`.")
    w(f"- **meshmud** {mp['MESHMUD_TAG']} (courtesy source pointer — its own code is MIT and its Go closure is permissive, so it is conveyed under MIT with the notices above; this entry becomes a GPL-3.0 source offer only if the build-time scan in `meshmud-build.sh` ever finds a Meshtastic protobuf module linked, recorded in the `.verdict` file): {mp['MESHMUD_REPO']}/tree/{mp['MESHMUD_TAG']} (commit `{mp['MESHMUD_COMMIT']}`, tarball sha256 `{mp['MESHMUD_TARBALL_SHA256']}`). Build recipe: `bootstrap/scripts/meshmud-build.sh`; module sources via the Go module proxy at the versions in `go.sum`.")
    w(f"- **meshtastic-python** {py['lock']['meshtastic'][0]} and its pip closure: https://pypi.org/project/meshtastic/{py['lock']['meshtastic'][0]}/ and https://github.com/meshtastic/python ; the exact files are pinned by hash in the lock.")
    w("- **meshtastic-presence** (our own reader, GPL-3.0-or-later): `os/messaging/sidecars/meshtastic-presence/` in this repository, at the same release tag as the binary that conveys it.")
    w("")

    w("## License texts")
    w("")
    for sid, url, note, body in texts:
        w(f"### {sid}")
        w("")
        w(f"Canonical: {url}. Source of this copy: {note}.")
        w("")
        w("```text")
        w(body)
        w("```")
        w("")
    w("### Silkscreen copyright line (OFL-1.1)")
    w("")
    w("```text")
    w(silk_line)
    w("```")
    w("")
    return "\n".join(L).rstrip("\n") + "\n", len(rust_rows), len(js_rows)


def main():
    ap = argparse.ArgumentParser(description="Generate os/THIRD-PARTY-NOTICES.md")
    ap.add_argument("--out", default=OUT_DEFAULT, help="output path (default: os/THIRD-PARTY-NOTICES.md)")
    ap.add_argument("--check", action="store_true",
                    help="only compare recorded lockfile SHA-256s with the current lockfiles; exit 1 if they differ")
    a = ap.parse_args()
    if a.check:
        check(a.out)
        return
    text, n_rust, n_js = render()
    with open(a.out, "w", encoding="utf-8") as f:
        f.write(text)
    print(f"wrote {a.out}: {n_rust} Rust crates, {n_js} JS packages")


if __name__ == "__main__":
    main()
