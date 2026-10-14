#!/usr/bin/env python3
"""gen-notices — build os/THIRD-PARTY-NOTICES.md from what we actually ship.

Python 3 standard library only (written to run on 3.8+). Offline: every cargo
call uses --locked --offline, pnpm reads the installed node_modules.

  python3 -I scripts/gen-notices.py            regenerate os/THIRD-PARTY-NOTICES.md
  python3 -I scripts/gen-notices.py --out F    write to F instead (for diffing)
  python3 -I scripts/gen-notices.py --check    fast gate: compare the lockfile SHA-256s
                                               recorded in the notices file with the
                                               current lockfiles; exit 1 when stale

Inputs
  Rust   `cargo metadata` (repository, authors, crate dirs) and
         `cargo tree -e normal --target <musl triple>` (the shipped set) in
         os/bootstrap (binary `111`, includes id111) and os/intelligence
         (binary `111-agent`). os/identity is not an input: id111 is built
         inside the bootstrap workspace and resolved by bootstrap/Cargo.lock.
  JS     `pnpm licenses list --prod --json` in os/interface/ui.
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


def die(msg):
    sys.stderr.write("gen-notices: " + msg + "\n")
    sys.exit(1)


def sha256(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


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
    stale = []
    for rel in LOCKFILES:
        path = os.path.join(OS_DIR, rel)
        if rel not in recorded:
            stale.append(f"  {rel}: not recorded in THIRD-PARTY-NOTICES.md")
        elif not os.path.isfile(path):
            stale.append(f"  {rel}: recorded but the file is gone")
        elif sha256(path) != recorded[rel]:
            stale.append(f"  {rel}: changed (recorded {recorded[rel][:12]}…, now {sha256(path)[:12]}…)")
    if stale:
        sys.stderr.write("THIRD-PARTY-NOTICES.md is stale — a lockfile changed since it was generated:\n")
        sys.stderr.write("\n".join(stale) + "\n")
        sys.stderr.write("Run: python3 -I os/bootstrap/scripts/gen-notices.py   (then review and commit the result)\n")
        sys.exit(1)
    print("THIRD-PARTY-NOTICES.md is current (lockfile hashes match)")


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
    w("- **Fonts, vendored code, embedded data:** see their sections.")
    w("- **Copyright lines** are taken from each component's license files (Rust: the unpacked crate in the cargo registry; JS: the installed package). Where")
    w("  a component's license files have no copyright line, its `authors` metadata is shown instead.")
    w("- **Choice of license.** Where a component offers a choice (`OR`), the text of any one offered license suffices. This file reproduces the MIT or Apache-2.0")
    w("  option wherever one is offered.")
    w("- Not covered here: separate programs that run next to our code and are not linked (Reticulum, Meshtastic, Termux and its packages, llama.cpp,")
    w("  opencode, RNode firmware). See `LICENSING.md` section 3. Their texts and source offers are added when a release carries them.")
    w("")
    w("Inputs at generation time (SHA-256):")
    w("")
    w("```")
    for rel in LOCKFILES:
        w(f"{sha256(os.path.join(OS_DIR, rel))}  {rel}")
    w("```")
    w("")
    w("Regenerate with `python3 -I os/bootstrap/scripts/gen-notices.py` whenever a lockfile changes. `--check` (run by `os/bootstrap/scripts/check.sh`) fails when the hashes above no longer match.")
    w("The output is sorted and contains no timestamps, so identical inputs give identical output.")
    w("")

    cnt = collections.Counter(r[0] for r in rust_rows)
    w("## Summary")
    w("")
    w("| Group | Components |")
    w("|---|---|")
    w(f"| Rust crates | {len(rust_rows)} |")
    w(f"| JavaScript packages | {len(js_rows)} |")
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
