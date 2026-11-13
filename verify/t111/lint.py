"""The seam map's lint (``python3 -I verify/run.py --lint``; options.md §3.5; catalog check ``x.seams.lint``).

It holds ``verify/seams.toml`` and ``verify/checks.toml`` to the tree. Eight rules fail, two things only warn. Every
rule is one function with a stable id (``RULES``); a finding names the rule, the subject (a seam id, a check id or a
path) and what is wrong.

Failing rules:

- ``ids``           an id is duplicated, or does not match its layers (a seam ``L3-L4:x`` joins layers 3 and 4, a check
                    ``l4.x`` touches layer 4, ``x.x`` layer ``X``; a check id with no ``l<N>.`` or ``x.`` prefix fails);
- ``files``         a side file, a check's script or test file (a path in its ``cmd``, or its ``cwd``; also the script a
                    ``sh -c "<script> ..."`` token starts with), a ``[[unmapped]]`` path or a spec file is missing, or a
                    ``§N.M`` reference matches no section of its file. A section is a heading line, or a bold lead (a
                    line that starts, after an optional ``- ``, with ``**9.4 The KB**``, ``**Binds:**``); a number
                    matches a heading carrying it or a lead starting with it, a name matches any heading or lead
                    containing it (``"<path> Updates & rollback"``). A ``planned`` seam is not held to the tree (its
                    files and spec may not exist yet);
- ``unknown-seam``  a check names a seam (``seams`` or ``phases``) that is not in the map;
- ``unclaimed``     a file matched by ``UNCLAIMED_GLOBS`` is claimed by no check and absent from ``[unmapped]``. A check
                    claims a file when a ``cmd`` token resolves to it (from the check's ``cwd``), to a directory holding it,
                    or when it is a ``cargo test`` whose ``--test`` names it (or, with no ``--test``, whose package or
                    manifest scope holds it; a ``--lib``, ``--doc``, ``--bin(s)`` or ``--example(s)`` run with no
                    ``--test`` claims no ``tests/*.rs``). A ``cmd`` token with ``*``, ``?`` or ``[`` is a pattern, not a
                    path;
- ``built-gap``     a ``built`` seam has no proving check, or (``GAP_NEEDS_R``) none of strength R, and no ``gap`` text;
- ``fake-mirror``   a ``[[fake]]`` names an unknown seam, a missing file, a mirrored side with no files, or an unknown
                    parity check;
- ``pi-floor``      a path under the root ``verify/`` is named by ``PI_FLOOR_PATHS`` (the release payload's manifest
                    generator and the cross build), or a ``PI_FLOOR_PATHS`` entry is missing from the tree: the test
                    system never ships on a node;
- ``known``         a ``[[known]]`` whose ``check`` names no check, whose ``test`` pattern starts with a check prefix that
                    matches no check, or whose ``source`` file does not exist.

One glob reading for every map path (a side file, an ``[unmapped]`` entry, ``UNCLAIMED_GLOBS``; ``glob_match`` and
``glob_paths``): ``*``, ``?`` and ``[..]`` stay inside one path segment, ``**`` as a whole segment crosses any number of
segments; a glob entry also matches everything below a directory it matches, as a plain directory entry does.

Warnings (``note:`` lines, never a failure): ``stub-marker`` (a ``stub`` seam none of whose side files carries the T0
marker, or a ``planned`` seam's side file has appeared: promote the row) and ``fake-parity`` (the count of fakes with
``parity = "none"``). A map that does not load fails as rule ``schema`` (the loader's own messages); duplicate ids are
reported under ``ids`` and dropped so the other rules still run.

The result is records in the runner's own form (``records()``: one PASS per rule that found nothing, one FAIL per finding,
ids ``x.seams.lint/<rule>[/<subject>]``), and ``main`` prints the same in the harness idiom (``== N. rule``,
``ok:`` / ``FAIL:`` / ``note:``, closing ``LINT GREEN`` or ``LINT: n FAIL``), so the exit/okfail adapters read it. Exit 0
green, 1 red.
"""

from __future__ import annotations

import argparse
import fnmatch
import re
import sys
import tomllib
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Mapping, NamedTuple

from .record import Record
from .schema import Check, MapError, SchemaError, SeamMap, load_map, parse_map

CHECK_ID = "x.seams.lint"
"""The catalog check that runs this lint."""

RULES: tuple[tuple[str, str], ...] = (
    ("ids", "ids are unique and match their layers"),
    ("files", "side files, check scripts and test files, spec files and sections exist"),
    ("unknown-seam", "every seam a check names is in the map"),
    ("unclaimed", "every verify script and test file is claimed by a check or listed in [unmapped]"),
    ("built-gap", "every built seam has a proving check of strength R or says its gap"),
    ("fake-mirror", "every [[fake]] names a real seam, file, mirrored side and parity check"),
    ("pi-floor", "nothing the release payload or the cross build names lives under verify/"),
    ("known", "every [[known]] names a real check and an existing source file"),
)
"""The failing rules in order: stable id, one line. The header of each block is ``== <n>. <id>``."""

SCHEMA_RULE = "schema"
"""The pseudo rule a map that does not load fails under."""

WARNINGS: tuple[str, ...] = ("stub-marker", "fake-parity")
"""The warning ids."""

UNCLAIMED_GLOBS: tuple[str, ...] = (
    "[0-9]-*/verify/*.sh",
    "[0-9]-*/verify/*/*.sh",
    "1-bootstrap/crates/node111/tests/*.rs",
    "5-messaging/crates/mesh111/tests/*.rs",
    "2-identity/crates/id111/tests/*.rs",
    "3-intelligence/crates/agent111/tests/*.rs",
    "4-interface/ui/test/*.test.ts",
    "[0-9]-*/sidecars/*/run-tests.sh",
)
"""Rule ``unclaimed``: the files that must be claimed by a check or listed in ``[unmapped]`` (repo-relative globs, read
as ``glob_paths`` reads them: ``*`` is one path segment; ``verify/*/*.sh`` is one level deeper than ``verify/*.sh``)."""

PI_FLOOR_PATHS: tuple[str, ...] = ("1-bootstrap/manifest", "1-bootstrap/scripts/cross-build.sh")
"""Rule ``pi-floor``: files and directories (searched recursively) that must not name any path under ``verify/``. An
entry missing from the tree fails the rule (a renamed generator would otherwise pass on nothing)."""

PI_FLOOR_MAX_BYTES = 2_000_000
"""A file larger than this is not searched by ``pi-floor`` (it is no script)."""

GAP_NEEDS_R = True
"""Rule ``built-gap``: a built seam needs ``gap`` text unless one of its proving checks is strength R (schema docstring of
``Seam.gap``). False = the literal reading of options.md §3.5 rule 5 (only a seam with no proving check at all)."""

T0_MARKER = re.compile(r"\bT0\b")
T0_MARKER_LINES = 12
"""A stub's side file carries ``T0`` within its first lines (``//! Layer 7 (§1): T0 stub; ...``)."""

SCRIPT_EXTS: tuple[str, ...] = (".sh", ".py", ".mjs", ".cjs", ".js", ".ts", ".rs", ".toml", ".json")
"""A ``cmd`` token ending in one of these is a file the check needs (it is reported when missing)."""

_PLACEHOLDER = re.compile(r"\{[a-z_]+\}")
_SECTION = re.compile(r"§\s*([A-Za-z]?\d+(?:\.\d+)*)")
_HEADING = re.compile(r"^#{1,6}\s+(.*?)\s*#*\s*$")
_BOLD_LEAD = re.compile(r"^\s*(?:[-*+]\s+)?\*\*(\S.*)$")
_KNOWN_LINE = re.compile(r":\d+(?:-\d+)?$")
_SEAM_ID_LAYERS = re.compile(r"^(?:(L[\d/L]+|X)-(L[\d/L]+|X)|X):")
_CHECK_ID_LAYER = re.compile(r"^(?:l(\d)|(x))\.")


@dataclass(frozen=True)
class Finding:
    """One lint finding: ``severity`` is ``fail`` or ``warn``; ``subject`` a seam id, check id or path."""

    rule: str
    severity: str
    subject: str
    message: str


@dataclass
class LintResult:
    """Every finding of one lint run, with the counts the ``ok:`` lines print."""

    findings: list[Finding] = field(default_factory=list)
    n_seams: int = 0
    n_checks: int = 0
    n_fakes: int = 0
    n_candidates: int = 0
    loaded: bool = True

    def fails(self, rule: str | None = None) -> list[Finding]:
        """The failing findings (of one rule when given)."""
        return [f for f in self.findings if f.severity == "fail" and (rule is None or f.rule == rule)]

    def warns(self, rule: str | None = None) -> list[Finding]:
        """The warnings (of one rule when given)."""
        return [f for f in self.findings if f.severity == "warn" and (rule is None or f.rule == rule)]

    def failing_rules(self) -> list[str]:
        """The ids of the rules with at least one failing finding, in rule order."""
        order = [SCHEMA_RULE] + [r for r, _ in RULES]
        return [r for r in order if self.fails(r)]


def _fail(rule: str, subject: str, message: str) -> Finding:
    return Finding(rule, "fail", subject, message)


def _warn(rule: str, subject: str, message: str) -> Finding:
    return Finding(rule, "warn", subject, message)


# ---------------------------------------------------------------------------------------------------------------------
# tree helpers


def has_glob(entry: str) -> bool:
    """True when a map path carries a glob character (``*``, ``?``, ``[``)."""
    return any(c in entry for c in "*?[")


def _match_segments(pattern: list[str], parts: list[str]) -> bool:
    if not pattern:
        return not parts
    head, rest = pattern[0], pattern[1:]
    if head == "**":
        return any(_match_segments(rest, parts[i:]) for i in range(len(parts) + 1))
    return bool(parts) and fnmatch.fnmatchcase(parts[0], head) and _match_segments(rest, parts[1:])


def glob_match(pattern: str, path: str) -> bool:
    """The one glob of the map: ``pattern`` matches the whole of ``path`` (both repo-relative, ``/``-separated).
    ``*``, ``?`` and ``[..]`` never cross a ``/``; a segment that is exactly ``**`` matches any number of segments
    (none too)."""
    return _match_segments([s for s in pattern.split("/") if s], [s for s in path.split("/") if s])


def glob_paths(root: Path, pattern: str) -> Iterable[Path]:
    """Every path under ``root`` that ``glob_match`` accepts for ``pattern`` (the same reading, walking the tree one
    segment at a time; a ``**`` descends through real directories only, never through a symlink)."""

    def walk(base: Path, segs: list[str]) -> Iterable[Path]:
        if not segs:
            if base.exists():
                yield base
            return
        head, rest = segs[0], segs[1:]
        if head == "**":
            yield from walk(base, rest)
            for child in _children(base):
                if child.is_dir() and not child.is_symlink():
                    yield from walk(child, segs)
                elif not rest:
                    yield child  # a trailing ** matches files too
        elif not has_glob(head):
            if (base / head).exists():
                yield from walk(base / head, rest)
        else:
            for child in _children(base):
                if fnmatch.fnmatchcase(child.name, head):
                    yield from walk(child, rest)

    yield from walk(root, [s for s in pattern.split("/") if s])


def _children(base: Path) -> list[Path]:
    try:
        return sorted(base.iterdir())
    except OSError:
        return []


def _candidates(root: Path) -> list[str]:
    """The repo-relative posix paths of every file ``UNCLAIMED_GLOBS`` matches, sorted."""
    found: set[str] = set()
    for pattern in UNCLAIMED_GLOBS:
        for path in glob_paths(root, pattern):
            if path.is_file():
                found.add(path.relative_to(root).as_posix())
    return sorted(found)


def _rel(path: Path, root: Path) -> str | None:
    """``path`` as a repo-relative posix string, or None when it lies outside ``root``."""
    try:
        return path.resolve().relative_to(root.resolve()).as_posix()
    except ValueError:
        return None


class Anchors(NamedTuple):
    """What a Markdown spec offers as sections: ``headings`` (the ``#`` lines) and ``leads`` (the bold lead-in of a line
    that starts with ``**``, optionally after ``- ``: ``**9.4 The KB**`` gives ``9.4 The KB``, ``**Binds:**`` gives
    ``Binds:``, ``- **Probation (the actor ...):** text`` gives ``Probation (the actor ...):``)."""

    headings: list[str]
    leads: list[str]


def _anchors(path: Path) -> Anchors:
    """The headings and bold leads of a Markdown file (lines outside code fences)."""
    out = Anchors([], [])
    fence = False
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.lstrip().startswith(("```", "~~~")):
            fence = not fence
            continue
        if fence:
            continue
        m = _HEADING.match(line)
        if m:
            out.headings.append(m.group(1))
            continue
        lead = _BOLD_LEAD.match(line)
        if lead:
            body = lead.group(1)
            end = body.find("**")
            out.leads.append(body[:end] if end >= 0 else body.split(":", 1)[0] + (":" if ":" in body else ""))
    return out


def split_spec(ref: str) -> tuple[str, str]:
    """``"<path> §4.2"`` -> ``("<path>", "§4.2")``; a bare path has an empty remainder."""
    parts = ref.strip().split(None, 1)
    if not parts:
        return "", ""
    return parts[0], parts[1].strip() if len(parts) > 1 else ""


def section_matches(heading: str, number: str) -> bool:
    """True when ``heading`` carries the section number ``number`` as a standalone token (``4.2`` in ``4.2 Link key``,
    also ``4.2.1``; never ``14.2`` or ``4.25``)."""
    return re.search(r"(?<![\w.])" + re.escape(number) + r"(?!\w)", heading) is not None


def lead_matches(lead: str, number: str) -> bool:
    """True when a bold lead starts with the section number ``number`` (``9.4`` in ``9.4 The KB``, ``§9.4``, also
    ``9.4.1``; never ``19.4`` or ``9.45``): a lead is a section anchor only by its first words, a number inside a
    sentence is not."""
    return re.match(r"\s*§?\s*" + re.escape(number) + r"(?!\w)", lead) is not None


def _unquote(text: str) -> str:
    return text.strip().strip("\"'`“”‘’ ").strip()


def _spec_problems(ref: str, root: Path, cache: dict[str, Anchors]) -> list[str]:
    """What is wrong with one spec reference (empty list = resolves)."""
    rel, rest = split_spec(ref)
    if not rel:
        return ["is empty"]
    path = root / rel
    if not path.is_file():
        return [f"spec file {rel} is missing"]
    if not rest:
        return []
    if rel not in cache:
        cache[rel] = _anchors(path) if rel.lower().endswith((".md", ".markdown")) else Anchors([], [])
    anchors = cache[rel]
    sections = _SECTION.findall(rest)
    problems: list[str] = []
    if sections:
        for number in sections:
            if not (any(section_matches(h, number) for h in anchors.headings)
                    or any(lead_matches(lead, number) for lead in anchors.leads)):
                problems.append(f"{rel} has no heading for §{number}")
        return problems
    wanted = " ".join(_unquote(rest).lower().split())
    if wanted and not any(wanted in " ".join(h.lower().split()) for h in (*anchors.headings, *anchors.leads)):
        problems.append(f"{rel} has no heading containing {_unquote(rest)!r}")
    return problems


# ---------------------------------------------------------------------------------------------------------------------
# what a check names and claims


def _tokens(check: Check) -> list[str]:
    """The ``cmd`` tokens that can name a file: before any bare ``--``, no placeholders, option values unpacked. A
    token with ``*``, ``?`` or ``[`` is a pattern for the tool (``test_*.py``), not a path, and is left out."""
    out: list[str] = []
    for token in check.cmd:
        if token == "--":
            break
        if token.startswith("-"):
            if "=" not in token or not token.startswith("--"):
                continue
            token = token.split("=", 1)[1]
        if not token or any(c.isspace() for c in token) or has_glob(token):
            continue
        out.append(token)
    return out


_SHELL_C = re.compile(r"^-[a-z]*c$")


def _shell_scripts(check: Check) -> list[str]:
    """The script a ``sh -c "<script> args"`` token starts with: the first word of a token that follows ``-c`` (or
    ``-lc``, ``-ec``), when it ends in ``SCRIPT_EXTS`` (a path that exists is checked too, below). Words with a
    placeholder or a glob character are left out."""
    out: list[str] = []
    cmd = list(check.cmd)
    if "--" in cmd:
        cmd = cmd[: cmd.index("--")]
    for before, token in zip(cmd, cmd[1:]):
        if not _SHELL_C.match(before) or not any(c.isspace() for c in token.strip()):
            continue
        words = token.split()
        first = words[0].strip("\"'") if words else ""
        if first and not has_glob(first) and not first.startswith(("-", "$")):
            out.append(first)
    return out


def _resolve(check: Check, token: str, root: Path) -> Path | None:
    """Where ``token`` points from the check's ``cwd`` (``{root}`` expanded), or None for a placeholder or a stray
    absolute path."""
    token = token.replace("{root}", str(root))
    if _PLACEHOLDER.search(token):
        return None
    path = Path(token)
    if path.is_absolute():
        return path if _rel(path, root) is not None else None
    return root / check.cwd / path


def check_files(check: Check, root: Path) -> list[tuple[str, Path]]:
    """The files a check needs: ``cmd`` tokens ending in ``SCRIPT_EXTS`` with the path each resolves to, and the script
    a ``sh -c "<script> ..."`` token starts with (when it ends in ``SCRIPT_EXTS`` or names a path that exists)."""
    out: list[tuple[str, Path]] = []
    for token in _tokens(check):
        if token.endswith(SCRIPT_EXTS):
            path = _resolve(check, token, root)
            if path is not None:
                out.append((token, path))
    for first in _shell_scripts(check):
        path = _resolve(check, first, root)
        if path is not None and (first.endswith(SCRIPT_EXTS) or path.exists()) and (first, path) not in out:
            out.append((first, path))
    return out


def _cargo_claims(check: Check, candidates: Iterable[str]) -> set[str]:
    """The candidate test files a ``cargo test`` check runs (by ``--test``, ``-p`` and the manifest scope)."""
    cmd = list(check.cmd)
    if not cmd or Path(cmd[0]).name != "cargo":
        return set()
    args = cmd[1:]
    if "--" in args:
        args = args[: args.index("--")]
    if not any(a in ("test", "t", "nextest") for a in args):
        return set()
    tests: list[str] = []
    pkgs: list[str] = []
    manifest = ""
    non_integration = False
    it = iter(args)
    for arg in it:
        if arg in ("--lib", "--doc", "--bin", "--bins", "--example", "--examples") or arg.startswith(
                ("--bin=", "--example=")):
            non_integration = True
        if arg == "--test":
            tests.append(next(it, ""))
        elif arg.startswith("--test="):
            tests.append(arg.split("=", 1)[1])
        elif arg in ("-p", "--package"):
            pkgs.append(next(it, ""))
        elif arg.startswith("--package="):
            pkgs.append(arg.split("=", 1)[1])
        elif arg == "--manifest-path":
            manifest = next(it, "")
        elif arg.startswith("--manifest-path="):
            manifest = arg.split("=", 1)[1]
    if non_integration and not tests and "--tests" not in args:
        return set()  # --lib, --doc, --bin(s), --example(s) run no tests/*.rs file
    scope = ""
    if manifest:
        parent = Path(check.cwd, manifest).parent.as_posix()
        scope = "" if parent == "." else parent
    elif check.cwd not in ("", "."):
        scope = Path(check.cwd).as_posix()
    if not (tests or pkgs or scope):
        return set()
    out: set[str] = set()
    for cand in candidates:
        path = Path(cand)
        if "tests" not in path.parts or path.suffix != ".rs":
            continue
        if tests and path.stem not in tests:
            continue
        if pkgs and not any(f"/{pkg}/" in f"/{cand}" for pkg in pkgs):
            continue
        if scope and not (cand == scope or cand.startswith(scope + "/")):
            continue
        out.add(cand)
    return out


def claimed_files(check: Check, root: Path, candidates: Iterable[str]) -> set[str]:
    """The repo-relative files ``check`` claims (see the module docstring)."""
    cands = list(candidates)
    claimed = _cargo_claims(check, cands)
    cwd_dir = (root / check.cwd).resolve()
    root_dir = root.resolve()
    for token in _tokens(check):
        path = _resolve(check, token, root)
        if path is None or not path.exists():
            continue
        rel = _rel(path, root)
        if rel is None:
            continue
        if path.is_file():
            claimed.add(rel)
        elif path.is_dir() and path.resolve() not in (root_dir, cwd_dir) and rel not in ("", "."):
            claimed.update(c for c in cands if c.startswith(rel + "/"))
    return claimed


def matches_entry(entry: str, path: str) -> bool:
    """A map path entry (``[unmapped]``, a seam's side file) matches a path: exactly, as a directory prefix, or as a
    glob in the one reading of ``glob_match`` (``*`` is one segment, ``**`` crosses segments); a glob also matches
    everything below a directory it matches."""
    entry = entry.rstrip("/")
    if path == entry or path.startswith(entry + "/"):
        return True
    if not has_glob(entry):
        return False
    parts = [p for p in path.split("/") if p]
    return any(glob_match(entry, "/".join(parts[:i])) for i in range(1, len(parts) + 1))


def proving_checks(seam_map: SeamMap, seam_id: str) -> list[Check]:
    """The checks whose results prove ``seam_id``: it is in their ``seams`` or in one of their ``phases``."""
    out = []
    for check in seam_map.checks:
        if seam_id in check.seams or any(seam_id in seams for seams in check.phases.values()):
            out.append(check)
    return out


# ---------------------------------------------------------------------------------------------------------------------
# the rules


def _id_layers(seam_id: str) -> tuple[frozenset[str], frozenset[str]] | None:
    """The two layer sets a seam id names (``L5/6-L7:x`` -> {5,6} and {7}); None for ``X:x`` (it names none)."""
    m = _SEAM_ID_LAYERS.match(seam_id)
    if not m or m.group(1) is None:
        return None

    def part(text: str) -> frozenset[str]:
        if text == "X":
            return frozenset({"X"})
        return frozenset(re.findall(r"\d", text))

    return part(m.group(1)), part(m.group(2))


def rule_ids(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Rule ``ids`` (the layer half; duplicates are found on the raw tables, see ``_duplicates``)."""
    out: list[Finding] = []
    for seam in seam_map.seams:
        want = _id_layers(seam.id)
        if want is None:
            continue
        have = (frozenset(str(x) for x in seam.a.layer), frozenset(str(x) for x in seam.b.layer))
        if {have[0], have[1]} != {want[0], want[1]}:
            out.append(_fail("ids", seam.id, f"the id names layers {_fmt(want[0])} and {_fmt(want[1])} but side a is "
                                              f"{_fmt(have[0])} and side b {_fmt(have[1])}"))
    for check in seam_map.checks:
        m = _CHECK_ID_LAYER.match(check.id)
        if not m:
            out.append(_fail("ids", check.id, "the id has no layer prefix (l<N>. for a layer, x. for the cross-layer "
                                              "system), so it cannot be held to layers = "
                                              f"{list(check.layers)}"))
            continue
        layer: Any = int(m.group(1)) if m.group(1) is not None else "X"
        if layer not in check.layers:
            out.append(_fail("ids", check.id, f"the id names layer {layer} but layers = {list(check.layers)}"))
    return out


def _fmt(layers: frozenset[str]) -> str:
    return "/".join(sorted(layers)) or "-"


def _duplicates(seams_doc: Mapping[str, Any], checks_doc: Mapping[str, Any]) -> tuple[list[Finding], dict, dict]:
    """Duplicate seam and check ids of the raw tables, and both documents with the later duplicates dropped."""
    findings: list[Finding] = []
    docs: list[dict[str, Any]] = []
    for doc, table, label in ((seams_doc, "seam", "seam"), (checks_doc, "check", "check")):
        seen: dict[str, int] = {}
        kept: list[Any] = []
        rows = doc.get(table, [])
        for index, row in enumerate(rows if isinstance(rows, list) else []):
            ident = row.get("id") if isinstance(row, dict) else None
            if isinstance(ident, str) and ident in seen:
                findings.append(_fail("ids", ident, f"duplicate {label} id ({table}[{index}] repeats "
                                                     f"{table}[{seen[ident]}])"))
                continue
            if isinstance(ident, str):
                seen[ident] = index
            kept.append(row)
        new = dict(doc)
        if table in doc:
            new[table] = kept
        docs.append(new)
    return findings, docs[0], docs[1]


def rule_files(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Rule ``files``: side files, check scripts and test files, ``[unmapped]`` paths, spec files and sections."""
    out: list[Finding] = []
    cache: dict[str, Anchors] = {}
    for seam in seam_map.seams:
        if seam.status == "planned":
            continue
        for name in ("a", "b"):
            for rel in seam.side(name).files:
                if not _entry_exists(rel, root):
                    out.append(_fail("files", seam.id, f"side {name} file {rel} is missing"))
        for ref in seam.spec:
            for problem in _spec_problems(ref, root, cache):
                out.append(_fail("files", seam.id, f"spec {ref!r}: {problem}"))
    for check in seam_map.checks:
        if not (root / check.cwd).is_dir():
            out.append(_fail("files", check.id, f"cwd {check.cwd} is not a directory"))
            continue
        for token, path in check_files(check, root):
            if not path.exists():
                out.append(_fail("files", check.id, f"cmd names {token}, which is missing (from cwd {check.cwd})"))
    for entry in seam_map.unmapped:
        if not _entry_exists(entry.path, root):
            out.append(_fail("files", "[unmapped]", f"{entry.path} matches nothing in the tree"))
    return out


def _entry_exists(entry: str, root: Path) -> bool:
    """A map path exists: a file or directory, or a glob that matches at least one path."""
    if has_glob(entry):
        return any(True for _ in glob_paths(root, entry))
    return (root / entry).exists()


def rule_unknown_seam(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Rule ``unknown-seam``: a check names a seam the map does not have."""
    out: list[Finding] = []
    for check in seam_map.checks:
        for seam_id in check.seams:
            if seam_map.seam(seam_id) is None:
                out.append(_fail("unknown-seam", check.id, f"seams names {seam_id!r}, which is not in seams.toml"))
        for prefix, seams in check.phases.items():
            for seam_id in seams:
                if seam_map.seam(seam_id) is None:
                    out.append(_fail("unknown-seam", check.id,
                                     f"phases[{prefix!r}] names {seam_id!r}, which is not in seams.toml"))
    return out


def rule_unclaimed(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Rule ``unclaimed``: a verify script or test file no check claims and ``[unmapped]`` does not list."""
    candidates = _candidates(root)
    claimed: set[str] = set()
    for check in seam_map.checks:
        claimed |= claimed_files(check, root, candidates)
    out: list[Finding] = []
    for path in candidates:
        if path in claimed:
            continue
        if any(matches_entry(u.path, path) for u in seam_map.unmapped):
            continue
        out.append(_fail("unclaimed", path, "no check claims this file and [unmapped] does not list it"))
    return out


def rule_built_gap(seam_map: SeamMap, root: Path, *, gap_needs_r: bool = GAP_NEEDS_R) -> list[Finding]:
    """Rule ``built-gap``: a built seam with no proving check (or none of strength R) and no ``gap`` text."""
    out: list[Finding] = []
    for seam in seam_map.seams:
        if seam.status != "built" or seam.gap.strip():
            continue
        proving = proving_checks(seam_map, seam.id)
        if not proving:
            out.append(_fail("built-gap", seam.id, "built, no check proves it and gap is empty"))
        elif gap_needs_r and not any(c.strength == "R" for c in proving):
            best = ", ".join(sorted({c.strength for c in proving}))
            out.append(_fail("built-gap", seam.id, f"built, no check of strength R (best: {best}) and gap is empty"))
    return out


def rule_fake_mirror(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Rule ``fake-mirror``: a ``[[fake]]`` whose seam, file, mirrored side or parity check does not exist."""
    out: list[Finding] = []
    for fake in seam_map.fakes:
        seam = seam_map.seam(fake.seam)
        if seam is None:
            out.append(_fail("fake-mirror", fake.path, f"seam {fake.seam!r} is not in seams.toml"))
        elif not seam.side(fake.mirrors).files:
            out.append(_fail("fake-mirror", fake.path, f"it mirrors side {fake.mirrors} of {fake.seam}, which lists no "
                                                        f"files"))
        if not (root / fake.path).exists():
            out.append(_fail("fake-mirror", fake.path, "the fake's file is missing"))
        if fake.parity != "none" and seam_map.check(fake.parity) is None:
            out.append(_fail("fake-mirror", fake.path, f"parity names no check: {fake.parity!r}"))
    return out


_PATH_CHAR = re.compile(r"[\w.\-]")
_VERIFY_WORD = re.compile(r"(?<![\w.\-])verify(?![\w\-])")
_JOIN_CONTEXT = re.compile(r"join\(|Path\(|/\s*[\"']verify[\"']")
_COPY_COMMAND = re.compile(r"^\s*(?:sudo\s+)?(?:cp|mv|rsync|tar|install|ln|zip)\s")
_COMPONENT = re.compile(r"[^\s/=:(,\"']+$")


def _root_prefix(before: str) -> bool:
    """True when ``before`` (the text ahead of ``verify``, ending in ``/``) names the repo root: ``./``, ``../``, a
    variable (``$ROOT/``, ``"${REPO}"/``, ``{root}/``) or a command substitution (``$(git rev-parse ...)/``)."""
    head = before[:-1].rstrip("\"'")
    comp = _COMPONENT.search(head)
    name = comp.group(0) if comp else ""
    if name in (".", "..") or "$" in name or name.endswith("}"):
        return True
    return head.endswith((")", "`")) and ("$(" in head or "`" in head)


def verify_hits(text: str) -> list[tuple[int, str]]:
    """The ``(line, snippet)`` of every mention of a path under the root ``verify/`` in ``text``.

    ``verify`` counts as a whole path part, in these forms: ``verify/x`` at the start of a word; after ``./``, ``../``, a
    variable, a quoted variable or a ``$(...)`` (``$ROOT/verify/x``, ``"$ROOT"/verify/x``, ``"$ROOT/verify"``,
    ``../verify``, ``$(git rev-parse --show-toplevel)/verify/x``); a quoted ``"verify"`` argument of ``os.path.join(...)``,
    ``Path(...)`` or ``/ "verify"``; the bare directory handed to ``cp``, ``mv``, ``rsync``, ``tar``, ``install``, ``ln``
    or ``zip``. ``4-interface/verify/x`` and ``my-verify/x`` (a layer's own verify folder, a longer word), a ``verify``
    in prose and a lone ``verify/`` are not hits.
    """
    hits: list[tuple[int, str]] = []
    for number, line in enumerate(text.splitlines(), 1):
        if any(_is_verify_path(line, m) for m in _VERIFY_WORD.finditer(line)):
            hits.append((number, line.strip()[:120]))
    return hits


def _is_verify_path(line: str, m: re.Match[str]) -> bool:
    """True when the ``verify`` word ``m`` found in ``line`` is the root ``verify/`` directory (see ``verify_hits``)."""
    before, after = line[: m.start()], line[m.end():]
    if after[:1] == "." and after[1:2] and (after[1].isalnum() or after[1] == "_"):
        return False  # verify.sh and the like: a file, not the directory
    if before.endswith("/"):
        return _root_prefix(before)
    if after[:1] == "/" and after[1:2] and _PATH_CHAR.match(after[1]):
        return True  # verify/x at the start of a word
    if before[-1:] in ("\"", "'") and after[:1] == before[-1:] and _JOIN_CONTEXT.search(line):
        anchor = max(before.rfind("join("), before.rfind("Path("))
        literals = before[anchor if anchor >= 0 else 0:][:-1]
        # join(root, "verify", ...) is the root verify/; join(root, "6-transport", "verify") is a layer's own
        return not re.search(r"[\"']", literals)
    return bool(_COPY_COMMAND.match(line)) and (not before.strip() or before[-1:].isspace() or before[-1:] in "\"'")


def rule_pi_floor(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Rule ``pi-floor``: ``PI_FLOOR_PATHS`` name nothing under the root ``verify/`` (and each exists)."""
    out: list[Finding] = []
    for entry in PI_FLOOR_PATHS:
        base = root / entry
        if not base.exists():
            out.append(_fail("pi-floor", entry, "is named by PI_FLOOR_PATHS but is missing from the tree (renamed or "
                                                "moved? update lint.PI_FLOOR_PATHS: the rule passes on nothing)"))
            continue
        files = [base] if base.is_file() else sorted(p for p in base.rglob("*") if p.is_file())
        for path in files:
            try:
                if path.stat().st_size > PI_FLOOR_MAX_BYTES:
                    continue
                text = path.read_bytes().decode("utf-8", "replace")
            except OSError:
                continue
            for number, snippet in verify_hits(text):
                rel = _rel(path, root) or str(path)
                out.append(_fail("pi-floor", f"{rel}:{number}", f"names a path under verify/ (never in the payload): "
                                                                f"{snippet}"))
    return out


def rule_known(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Rule ``known``: a ``[[known]]`` that can never match (a ``vacuous`` entry with a typo keeps a PASS that proves
    nothing) or whose source is gone: ``check`` names no check, the check prefix of ``test`` (the text before its first
    ``/``, a glob allowed) matches no check, ``source`` (``<file>[:line]``, first word) is not a file of the tree."""
    out: list[Finding] = []
    ids = [c.id for c in seam_map.checks]
    for known in seam_map.known:
        if known.check and seam_map.check(known.check) is None:
            out.append(_fail("known", known.test, f"check names {known.check!r}, which is not in checks.toml"))
        prefix = known.test.split("/", 1)[0]
        if not any(fnmatch.fnmatchcase(i, prefix) for i in ids):
            out.append(_fail("known", known.test, f"the check prefix {prefix!r} of test matches no check in "
                                                  f"checks.toml"))
        words = known.source.split()
        source = _KNOWN_LINE.sub("", words[0]) if words else ""
        if not source or not (root / source).is_file():
            out.append(_fail("known", known.test, f"source {known.source!r}: {source or 'no path'} is not a file of "
                                                  f"the tree"))
    return out


RULE_FUNCS = {
    "ids": rule_ids,
    "files": rule_files,
    "unknown-seam": rule_unknown_seam,
    "unclaimed": rule_unclaimed,
    "built-gap": rule_built_gap,
    "fake-mirror": rule_fake_mirror,
    "pi-floor": rule_pi_floor,
    "known": rule_known,
}
"""Rule id -> function ``(seam_map, root) -> list[Finding]``, one per entry of ``RULES``."""


def warn_stub_marker(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Warning ``stub-marker``: a planned seam's side file appeared, or a stub seam none of whose side files carries
    the T0 marker (a mixed seam, one side built and the other still a T0 stub, is a stub that keeps its marker)."""
    out: list[Finding] = []
    for seam in seam_map.seams:
        if seam.status not in ("stub", "planned"):
            continue
        present = [(name, rel) for name in ("a", "b") for rel in seam.side(name).files if (root / rel).is_file()]
        if seam.status == "planned":
            out.extend(_warn("stub-marker", seam.id, f"planned, but side {name} file {rel} exists: promote the row "
                                                     f"(stub or built)") for name, rel in present)
            continue
        if not present:
            continue
        marked = False
        for _, rel in present:
            head = (root / rel).read_text(encoding="utf-8", errors="replace").splitlines()[:T0_MARKER_LINES]
            if any(T0_MARKER.search(line) for line in head):
                marked = True
                break
        if not marked:
            shown = ", ".join(f"side {name} {rel}" for name, rel in present)
            out.append(_warn("stub-marker", seam.id, f"stub, but no side file carries a T0 marker in its first lines "
                                                      f"({shown}): promote the row"))
    return out


def warn_fake_parity(seam_map: SeamMap, root: Path) -> list[Finding]:
    """Warning ``fake-parity``: how many fakes are checked against nothing they mirror."""
    none = [f for f in seam_map.fakes if f.parity == "none"]
    if not none:
        return []
    return [_warn("fake-parity", "[[fake]]", f"{len(none)} of {len(seam_map.fakes)} fake(s) checked against nothing "
                                             f"they mirror (parity = \"none\")")]


# ---------------------------------------------------------------------------------------------------------------------
# running the lint


def lint_map(seam_map: SeamMap, root: Path, *, gap_needs_r: bool = GAP_NEEDS_R) -> LintResult:
    """Run every rule and warning on a loaded map against the tree at ``root``."""
    result = LintResult(n_seams=len(seam_map.seams), n_checks=len(seam_map.checks), n_fakes=len(seam_map.fakes),
                        n_candidates=len(_candidates(root)))
    for rule, _ in RULES:
        if rule == "built-gap":
            result.findings.extend(rule_built_gap(seam_map, root, gap_needs_r=gap_needs_r))
        else:
            result.findings.extend(RULE_FUNCS[rule](seam_map, root))
    result.findings.extend(warn_stub_marker(seam_map, root))
    result.findings.extend(warn_fake_parity(seam_map, root))
    return result


def lint_docs(seams_doc: Mapping[str, Any], checks_doc: Mapping[str, Any], root: Path, *,
              gap_needs_r: bool = GAP_NEEDS_R, seams_file: str = "verify/seams.toml",
              checks_file: str = "verify/checks.toml") -> LintResult:
    """Lint two parsed TOML documents: duplicates first, then the schema, then every rule on the loaded map."""
    dups, seams_doc2, checks_doc2 = _duplicates(seams_doc, checks_doc)
    try:
        seam_map = parse_map(seams_doc2, checks_doc2, seams_file=seams_file, checks_file=checks_file)
    except SchemaError as exc:
        result = LintResult(loaded=False)
        result.findings.extend(dups)
        result.findings.extend(_fail(SCHEMA_RULE, _schema_subject(e), str(e)) for e in exc.errors)
        return result
    result = lint_map(seam_map, root, gap_needs_r=gap_needs_r)
    result.findings[:0] = dups
    return result


def _schema_subject(error: MapError) -> str:
    return error.id or (f"{error.table}[{error.index}]" if error.table and error.index is not None else error.file)


def lint_dir(map_dir: Path, root: Path, *, gap_needs_r: bool = GAP_NEEDS_R) -> LintResult:
    """Lint ``<map_dir>/seams.toml`` and ``<map_dir>/checks.toml`` against the tree at ``root``."""
    docs: list[dict[str, Any]] = []
    errors: list[Finding] = []
    for name in ("seams.toml", "checks.toml"):
        path = map_dir / name
        shown = _rel(path, root) or str(path)
        try:
            with path.open("rb") as fh:
                docs.append(tomllib.load(fh))
        except FileNotFoundError:
            errors.append(_fail(SCHEMA_RULE, shown, "file not found"))
            docs.append({})
        except tomllib.TOMLDecodeError as exc:
            errors.append(_fail(SCHEMA_RULE, shown, f"not valid TOML: {exc}"))
            docs.append({})
    if errors:
        result = LintResult(loaded=False)
        result.findings.extend(errors)
        return result
    return lint_docs(docs[0], docs[1], root, gap_needs_r=gap_needs_r,
                     seams_file=_rel(map_dir / "seams.toml", root) or "seams.toml",
                     checks_file=_rel(map_dir / "checks.toml", root) or "checks.toml")


# ---------------------------------------------------------------------------------------------------------------------
# records and text


def _slug(text: str) -> str:
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-")[:60] or "x"


def records(result: LintResult) -> list[Record]:
    """The lint as runner records: a PASS ``x.seams.lint/<rule>`` per rule with no failure, a FAIL
    ``x.seams.lint/<rule>/<subject>`` per failing finding (the finding's words in ``assertion``)."""
    out: list[Record] = []
    seen: dict[str, int] = {}
    rules = [(SCHEMA_RULE, "the map loads")] + list(RULES) if not result.loaded else list(RULES)
    for rule, _ in rules:
        fails = result.fails(rule)
        if not fails:
            if result.loaded or rule == SCHEMA_RULE:
                out.append(Record(id=f"{CHECK_ID}/{rule}", status="PASS", phase=rule, check=CHECK_ID))
            continue
        for finding in fails:
            base = f"{CHECK_ID}/{rule}/{_slug(finding.subject)}"
            seen[base] = seen.get(base, 0) + 1
            rid = base if seen[base] == 1 else f"{base}-{seen[base]}"
            out.append(Record(id=rid, status="FAIL", assertion=f"{finding.subject}: {finding.message}", phase=rule,
                              check=CHECK_ID))
    return out


def render(result: LintResult) -> str:
    """The text of ``--lint``: harness idiom, deterministic, ending ``LINT GREEN`` or ``LINT: n FAIL``."""
    lines: list[str] = []
    if not result.loaded or result.fails(SCHEMA_RULE):
        lines.append(f"== 0. {SCHEMA_RULE}")
        fails = result.fails(SCHEMA_RULE)
        for f in fails:
            lines.append(f"FAIL: {f.rule}: {f.message}")
    for number, (rule, text) in enumerate(RULES, 1):
        if not result.loaded and not result.fails(rule):
            continue
        lines.append(f"== {number}. {rule}")
        fails = result.fails(rule)
        if not fails:
            lines.append(f"ok: {rule}: {_ok_text(rule, text, result)}")
        for f in fails:
            lines.append(f"FAIL: {f.rule}: {f.subject}: {f.message}")
    warns = result.warns()
    for w in warns:
        lines.append(f"note: warn {w.rule}: {w.subject}: {w.message}")
    n_fail = len(result.fails())
    lines.append("LINT GREEN" if not n_fail else f"LINT: {n_fail} FAIL")
    return "\n".join(lines) + "\n"


def _ok_text(rule: str, text: str, result: LintResult) -> str:
    counts = {"ids": f"{result.n_seams} seams, {result.n_checks} checks",
              "unclaimed": f"{result.n_candidates} verify and test files",
              "fake-mirror": f"{result.n_fakes} fakes"}
    extra = counts.get(rule)
    return f"{text} ({extra})" if extra else text


def load_map_reporting(map_dir: Path, root: Path, command: str) -> SeamMap | None:
    """Load the map for a map command; print every schema error to stderr and return None when it does not load."""
    try:
        return load_map(map_dir, display_root=root)
    except SchemaError as exc:
        for error in exc.errors:
            print(f"map: {error}", file=sys.stderr)
        print(f"--{command}: the map does not load ({len(exc.errors)} schema error(s)); run --lint", file=sys.stderr)
        return None


def main(args: argparse.Namespace, root: Path, map_dir: Path) -> int:
    """``--lint``: print the lint of the map in ``map_dir`` against the tree at ``root``; exit 0 green, 1 red."""
    result = lint_dir(map_dir, root)
    sys.stdout.write(render(result))
    return 1 if result.fails() else 0
