#!/usr/bin/env python3
# Licence: pending the owner's contact with upstream (LICENSE-NOTE.md); no licence is granted by this file yet.
"""gen_lock: produce / verify lxmf-sidecar-lock.txt (hash-locked pip closure of rns + lxmf, spec §6).

This is how the lock is made, the same way ``rns-lock.txt`` is (a plain pip requirements
file, every line ``name==version`` plus every published file hash, installed with
``pip install --require-hashes -r <lock>``; nothing is vendored). The same bytes are written to
the component copy ``1-bootstrap/crates/node111/assets/components/lxmf.lock.txt`` (embedded in
the signed binary beside ``lxmf.toml``; ``111 install`` downloads exactly those hashes into
``cache/packages/lxmf/`` and the venv is built from that cache):

  python3 gen_lock.py --pip "python3.11 -m pip"                 # needs network, run at release time
  python3 gen_lock.py --verify                                  # offline: parse + structural checks

``requirements.in`` pins ``rns`` and ``lxmf`` exactly; ``rns`` must be the version the Layer 1
``rns-lock.txt`` pins at release time (two ``rns`` copies share one instance protocol, spec §6).

How it resolves: ``pip install --dry-run --report`` on ``requirements.in`` (the closure
for THIS interpreter), then for every package the sha256 of EVERY file PyPI publishes for
that exact version (wheels for every platform and the sdist, like ``pip-compile
--generate-hashes``), so the same lock installs on x86_64, aarch64 and musl. A package
that every dependency edge reaches only through an environment marker (for example
``async-timeout; python_version < "3.11"``) keeps that marker; dropping a parent's
condition only ever widens the set, never narrows it. Resolve on the OLDEST available
CPython (3.11 on the build box) so newer interpreters get a subset.

Python standard library only. Writes nothing but ``--out``.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
import time
import urllib.request
from typing import Dict, List, Optional, Tuple

HERE = os.path.dirname(os.path.abspath(__file__))
DEFAULT_IN = os.path.join(HERE, "requirements.in")
DEFAULT_OUT = os.path.join(HERE, "lxmf-sidecar-lock.txt")
DEFAULT_COMPONENT = os.path.normpath(os.path.join(
    HERE, "..", "..", "..", "1-bootstrap", "crates", "node111", "assets", "components", "lxmf.lock.txt"))
PYPI = "https://pypi.org/pypi/%s/%s/json"

NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*")
LOCK_LINE_RE = re.compile(r"^([a-z0-9][a-z0-9._-]*)==([A-Za-z0-9][A-Za-z0-9.!+_-]*)(?: ; (.+?))?(?: \\)?$")
HASH_RE = re.compile(r"^\s+--hash=sha256:([0-9a-f]{64})(?: \\)?$")


def canon(name: str) -> str:
    return re.sub(r"[-_.]+", "-", name).lower()


# --------------------------------------------------------------------------- verify
def parse_lock(text: str) -> Dict[str, dict]:
    """Parse a lock; raises ValueError on anything that ``--require-hashes`` would refuse.

    Returns ``{name: {"version", "marker", "hashes": [...]}}``.
    """
    out: Dict[str, dict] = {}
    cur: Optional[dict] = None
    for no, raw in enumerate(text.splitlines(), 1):
        line = raw.rstrip()
        if not line or line.lstrip().startswith("#"):
            continue
        m = HASH_RE.match(line)
        if m:
            if cur is None:
                raise ValueError("line %d: hash without a requirement" % no)
            cur["hashes"].append(m.group(1))
            continue
        m = LOCK_LINE_RE.match(line)
        if not m:
            raise ValueError("line %d: not a pinned requirement: %r" % (no, line))
        name = m.group(1)
        if name in out:
            raise ValueError("line %d: duplicate requirement %s" % (no, name))
        cur = {"version": m.group(2), "marker": m.group(3), "hashes": []}
        out[name] = cur
    for name, entry in out.items():
        if not entry["hashes"]:
            raise ValueError("%s has no hashes" % name)
        if len(set(entry["hashes"])) != len(entry["hashes"]):
            raise ValueError("%s repeats a hash" % name)
    if not out:
        raise ValueError("empty lock")
    return out


def read_requirements_in(path: str) -> List[str]:
    reqs = []
    with open(path, encoding="utf-8") as f:
        for raw in f:
            line = raw.split("#", 1)[0].strip()
            if line:
                reqs.append(line)
    return reqs


def verify(lock_path: str, in_path: str, component_path: Optional[str] = None) -> Dict[str, dict]:
    with open(lock_path, encoding="utf-8") as f:
        text = f.read()
    lock = parse_lock(text)
    if component_path and os.path.exists(component_path):
        with open(component_path, encoding="utf-8") as f:
            if f.read() != text:
                raise ValueError("%s differs from %s: regenerate with gen_lock.py" % (component_path, lock_path))
    for req in read_requirements_in(in_path):
        m = re.match(r"^([A-Za-z0-9._-]+)==([^\s;]+)$", req)
        if not m:
            raise ValueError("requirements.in must hold exact pins only, got %r" % req)
        name = canon(m.group(1))
        if name not in lock or lock[name]["version"] != m.group(2):
            raise ValueError("lock does not pin %s==%s" % (name, m.group(2)))
    return lock


# --------------------------------------------------------------------------- generate
def run_pip_report(pip_cmd: List[str], in_path: str) -> List[dict]:
    with tempfile.TemporaryDirectory() as tmp:
        report = os.path.join(tmp, "report.json")
        cmd = pip_cmd + ["install", "--dry-run", "--ignore-installed", "--quiet", "--no-input",
                         "--disable-pip-version-check", "--report", report, "-r", in_path]
        subprocess.run(cmd, check=True)
        with open(report, encoding="utf-8") as f:
            return json.load(f)["install"]


def split_requirement(req: str) -> Tuple[str, Optional[str]]:
    """``"dbus-fast>=1; platform_system == 'Linux'"`` to ``("dbus-fast", marker-or-None)``."""
    body, _, marker = req.partition(";")
    m = NAME_RE.match(body.strip())
    if not m:
        raise ValueError("cannot parse requirement %r" % req)
    marker = marker.strip() or None
    return canon(m.group(0)), marker


def edges(installs: List[dict]) -> Dict[str, List[Tuple[str, Optional[str]]]]:
    """``{dep: [(parent, marker-or-None), ...]}`` for the dependency edges inside the closure."""
    names = {canon(i["metadata"]["name"]) for i in installs}
    out: Dict[str, List[Tuple[str, Optional[str]]]] = {n: [] for n in names}
    for item in installs:
        parent = canon(item["metadata"]["name"])
        for req in item["metadata"].get("requires_dist") or []:
            dep, marker = split_requirement(req)
            if marker and "extra" in marker:
                continue  # we ask for no extras
            if dep in names:
                out[dep].append((parent, marker))
    return out


def marker_for(incoming: List[Tuple[str, Optional[str]]]) -> Optional[str]:
    """The marker to keep: only when EVERY incoming edge is conditional."""
    if not incoming or any(m is None for _, m in incoming):
        return None
    uniq = sorted({m for _, m in incoming if m})
    if len(uniq) == 1:
        return uniq[0]
    return " or ".join("(%s)" % m for m in uniq)


def pypi_hashes(name: str, version: str) -> List[str]:
    last: Optional[Exception] = None
    for attempt in range(4):
        try:
            with urllib.request.urlopen(PYPI % (name, version), timeout=60) as resp:
                data = json.load(resp)
            files = [u for u in data["urls"] if not u.get("yanked")]
            hashes = sorted({u["digests"]["sha256"] for u in files})
            if not hashes:
                raise ValueError("PyPI lists no files for %s==%s" % (name, version))
            return hashes
        except Exception as exc:  # network flake: retry with back-off
            last = exc
            time.sleep(1 + attempt)
    raise SystemExit("cannot read hashes for %s==%s from PyPI: %s" % (name, version, last))


def render(installs: List[dict], pip_version: str) -> str:
    inc = edges(installs)
    roots = {canon(i["metadata"]["name"]) for i in installs if i.get("requested")}
    lines = [
        "# lxmf-sidecar-lock.txt: hash-locked pip closure of requirements.in (rns + lxmf, the LXMF helper venv, spec 6).",
        "# Generated by gen_lock.py (pip %s); do not edit by hand." % pip_version,
        "# Install: pip install --require-hashes -r lxmf-sidecar-lock.txt  (copy: node111 assets/components/lxmf.lock.txt)",
        "# Every file PyPI publishes for each pinned version is hashed (all platforms + sdist).",
        "",
    ]
    for item in sorted(installs, key=lambda i: canon(i["metadata"]["name"])):
        meta = item["metadata"]
        name, version = canon(meta["name"]), meta["version"]
        marker = None if name in roots else marker_for(inc.get(name, []))
        via = sorted({p for p, _ in inc.get(name, [])})
        lines.append("# via " + (", ".join(via) if via else "requirements.in"))
        head = "%s==%s" % (name, version) + (" ; " + marker if marker else "")
        hashes = pypi_hashes(name, version)
        lines.append(head + " \\")
        for i, h in enumerate(hashes):
            lines.append("    --hash=sha256:" + h + (" \\" if i < len(hashes) - 1 else ""))
        lines.append("")
    return "\n".join(lines).rstrip("\n") + "\n"


def pip_version_of(pip_cmd: List[str]) -> str:
    out = subprocess.run(pip_cmd + ["--version"], check=True, stdout=subprocess.PIPE,
                         universal_newlines=True).stdout
    m = re.match(r"pip (\S+).*\(python (\S+)\)", out)
    return "%s, CPython %s" % (m.group(1), m.group(2)) if m else "unknown"


def main(argv: Optional[List[str]] = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--in", dest="in_path", default=DEFAULT_IN, help="requirements.in (exact pins)")
    ap.add_argument("--out", default=DEFAULT_OUT, help="lock file to write / verify")
    ap.add_argument("--component-out", default=DEFAULT_COMPONENT,
                    help="the embedded copy beside lxmf.toml (same bytes); '' skips it")
    ap.add_argument("--pip", default="python3 -m pip", help="how to run pip (a command line)")
    ap.add_argument("--verify", action="store_true", help="offline structural check of --out, then exit")
    args = ap.parse_args(argv)
    try:
        if args.verify:
            lock = verify(args.out, args.in_path, args.component_out or None)
            print("ok: %d packages, %d hashes" % (len(lock), sum(len(e["hashes"]) for e in lock.values())))
            return 0
        pip_cmd = shlex.split(args.pip)
        installs = run_pip_report(pip_cmd, args.in_path)
        text = render(installs, pip_version_of(pip_cmd))
        parse_lock(text)  # never write a lock we would refuse ourselves
        tmp = args.out + ".tmp"
        with open(tmp, "w", encoding="utf-8") as f:
            f.write(text)
        os.replace(tmp, args.out)
        if args.component_out:
            ctmp = args.component_out + ".tmp"
            with open(ctmp, "w", encoding="utf-8") as f:
                f.write(text)
            os.replace(ctmp, args.component_out)
        verify(args.out, args.in_path, args.component_out or None)
        print("wrote %s" % args.out)
        if args.component_out:
            print("wrote %s" % args.component_out)
        return 0
    except (ValueError, OSError, subprocess.CalledProcessError) as exc:
        sys.stderr.write("gen_lock: %s\n" % exc)
        return 1


if __name__ == "__main__":
    sys.exit(main())
