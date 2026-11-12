"""The adapters: how a check's log becomes result records (options.md §3.4).

The contract. An adapter is ``fn(check, log, status, ctx) -> list[Record]``, registered with ``@register("<name>")``:

- ``check``: the ``t111.schema.Check`` that ran.
- ``log``: the path of the check's combined, ordered log (stdout and stderr in one stream, ``out.log``).
- ``status``: the exit status (negative = killed by that signal, as ``subprocess`` reports it).
- ``ctx``: an ``AdapterContext`` (below).

It returns bare ``t111.record.Record`` objects: ``id`` (``<check>`` or ``<check>/<phase>/<slug>``), ``status``, and
whatever it can read: ``reason``, ``assertion`` (verbatim), ``log_line``, ``phase``, ``seams`` (from ``check.phases``),
``after``, ``test``, ``rerun``, ``kept``, and ``check`` (a covered unit's id, for records a gate's output yields for
that unit). The engine fills everything else from the map. The engine's rules hold whatever an adapter returns:

- a timeout always yields a FAIL record with the reason ``timeout ...``;
- a non-zero exit with no FAIL record always yields the FAIL ``exit <n> but no failing result was parsed`` (a lying
  parse never turns red into green), PASS and SKIPPED records notwithstanding, unless the adapter sets
  ``ctx.exit_declared = True``. The rule is applied twice: on the adapter's records, and again after ``[[known]]``
  (a known FAIL turned SKIPPED never hides the exit). An adapter sets ``exit_declared`` only when its parse accounts
  for this exact exit: libtest when every test binary finished and the failed count of every ``test result:`` line
  equals the FAIL records it parsed (cargo's 101); a harness whose documented skip code is 77 and whose log says why
  it skipped (``exit77``). Never for "some tests were ignored";
- a check that could not start, or that the runner killed on an interrupt, yields one FAIL with the check's id;
- ``[[known]]`` (F7) applies only to the records an adapter parsed from a test's own output: every record the engine
  makes (the rules above, skips before the start, lock waits, hygiene) is never eligible, and neither is the record
  of the ``exit`` and ``exit77`` adapters (it is the exit status, not a parsed test: those two adapters, and only
  they, set ``Record.engine``);
- the engine keeps one record per check id: a rule above replaces the adapter's record of the check's own id;
- the engine links a cascade: every FAIL after the first FAIL of one harness run carries ``after`` (read that first).

The record ids (stable between a gate run and a unit run, so the ``[[known]]`` patterns ``*/<test fn>`` apply):

- **libtest**: ``<unit>/<test name as libtest prints it>`` (``l1.node111.phone_flow/preflight_happy``,
  ``l2.id111.lib/keyset::tests::derive``). In a unit run the unit is the check itself. In a gate run each test binary
  is attributed to the unit that covers it: the cargo line ``Running tests/<stem>.rs (…)`` names the unit whose cmd
  has ``--test <stem>``, ``Running unittests src/lib.rs (…/deps/<crate>-<hash>)`` the unit with ``-p <crate> --lib``
  (a unit's own test filter, ``--lib relay::``, wins for the tests it matches; an ambiguous name prefers the units the
  running gate, or the gate a nested stage runs, covers). A test no unit covers keeps the gate:
  ``<gate>/<stage>/<binary>/<test>`` (``l5.gate/test/doc/…`` for doc tests). A test binary that did not finish is one
  FAIL with the unit's own id (``<unit>``, or ``<gate>/<stage>/<binary>``), or, when the binary died while one named
  test ran, that test's FAIL.
- **vitest**: ``<unit>/<describe> > <test>``; ``<unit>`` alone for a file-level result (a file that passed with no
  per-test lines, a file that failed to load); the unit is the one whose cmd names the test file.
- **pytest / unittest**: ``<owner>/<module.Class.test>`` or ``<owner>/<file>::<test>``; the owner is the unit, or the
  covered unit a gate stage runs when exactly one matches, else ``<gate>/<stage>``.
- **okfail**: ``<check>/<phase>/<slug of the description>`` (phase = its number, ``5a``; the slug cut at `` (`` and
  ``  [``, long numbers and hex as ``n``), ``<check>`` for the harness-level verdicts.
- **stagewise**: ``<gate>/<stage>`` for a stage's own verdict, ``<gate>/<stage>/<…>`` for an inner result nobody else
  owns; nested stages are ``messaging/test``.
- **table**: ``<check>/<file>``; **buildpy**: ``<check>/<path>:<line>/<code>``.

A repeated id within one check gets ``#2``, ``#3``.

``log_tail`` is the helper for a tail: it reads from the end of the log (never the whole file into memory), keeps the
last lines within the 2 KB of an ``assertion`` (clipped from the end on a line boundary) and returns the 1-based line
where the kept text starts. The parsers read the log once, line by line (ANSI colour codes stripped, a carriage-return
overwrite keeps its last text), and hold at most one stage's last 40 lines and one failure block at a time.
"""

from __future__ import annotations

import json
import os
import re
from collections import deque
from dataclasses import dataclass, field, replace
from pathlib import Path
from typing import Callable, Iterable, Iterator

from .record import ASSERTION_MAX_BYTES, Record, clip
from .schema import Check, SeamMap

TAIL_LINES = 40
"""How many closing log lines a FAIL record of the ``exit`` adapter (or a failed stage) carries."""


@dataclass
class AdapterContext:
    """What an adapter may read besides the log, and the one thing it may answer back.

    - ``root``: the repo root; ``scratch``: the check's scratch dir; ``tmp``: its ``TMPDIR``.
    - ``results``: the ``T111_RESULTS`` file the check may have written (X6 emitter), maybe absent.
    - ``timed_out``: the check hit ``timeout_s`` and was killed.
    - ``seam_map``: the loaded map (for ``check.phases`` and covered units).
    - ``facts``: run facts (``euid0``, ``offline``) an adapter may need for a skip reason.
    - ``exit_declared``: False until the adapter sets it; True only when the adapter's records explain this exact
      non-zero exit status (see the module docstring). The engine reads it after the adapter returns.
    """

    root: Path
    scratch: Path
    tmp: Path
    results: Path
    timed_out: bool
    seam_map: SeamMap
    facts: dict[str, bool] = field(default_factory=dict)
    exit_declared: bool = False


Adapter = Callable[[Check, Path, int, AdapterContext], list[Record]]
"""The adapter signature (see the module docstring)."""

REGISTRY: dict[str, Adapter] = {}
"""Adapter name (``check.adapter``) -> function. Filled at import by ``@register``; never changed afterwards."""


def register(name: str) -> Callable[[Adapter], Adapter]:
    """Decorator: register ``fn`` under ``name`` (a name is registered once)."""

    def wrap(fn: Adapter) -> Adapter:
        if name in REGISTRY:
            raise ValueError(f"adapter {name!r} registered twice")
        REGISTRY[name] = fn
        return fn

    return wrap


def get(name: str) -> Adapter | None:
    """The adapter registered under ``name``, or None."""
    return REGISTRY.get(name)


TAIL_WINDOW = ASSERTION_MAX_BYTES + 4096
"""How many closing bytes of a log ``log_tail`` reads (the kept tail is at most ``ASSERTION_MAX_BYTES``)."""


def count_lines(log: Path, end: int) -> int:
    """The number of newlines in the first ``end`` bytes of ``log`` (read in chunks, never whole)."""
    total = 0
    with log.open("rb") as fh:
        left = end
        while left > 0:
            chunk = fh.read(min(left, 1 << 20))
            if not chunk:
                break
            total += chunk.count(b"\n")
            left -= len(chunk)
    return total


def log_tail(log: Path, lines: int = TAIL_LINES, limit: int = ASSERTION_MAX_BYTES) -> tuple[str, int]:
    """The last ``lines`` lines of ``log``, clipped from the end to ``limit`` bytes on a line boundary, and the 1-based
    line number where the kept text starts (0 for an empty or missing log).

    The file is read from its end (``TAIL_WINDOW`` bytes) and the line number counted in chunks. When the last line
    alone is longer than ``limit``, its last ``limit`` bytes are kept (cut on a UTF-8 boundary).
    """
    try:
        fh = log.open("rb")
    except FileNotFoundError:
        return "", 0
    with fh:
        size = os.fstat(fh.fileno()).st_size
        if size == 0:
            return "", 0
        start = max(0, size - max(TAIL_WINDOW, limit + 1))
        fh.seek(start)
        data = fh.read(size - start)
    body = data[:-1] if data.endswith(b"\n") else data
    pieces = body.split(b"\n")
    offsets: list[int] = []
    pos = start
    for piece in pieces:
        offsets.append(pos)
        pos += len(piece) + 1
    first = 0
    if start > 0 and len(pieces) > 1:
        first = 1  # the window began inside a line: drop that partial line
    first = max(first, len(pieces) - lines)
    kept = pieces[first:]
    kept_offsets = offsets[first:]
    total = 0
    cut = len(kept)
    for index in range(len(kept) - 1, -1, -1):
        total += len(kept[index]) + (1 if index < len(kept) - 1 else 0)
        if total > limit:
            break
        cut = index
    if cut == len(kept):  # the last line alone exceeds the limit: keep its end
        tail_bytes = kept[-1][-limit:]
        text = tail_bytes.decode("utf-8", "ignore") if len(kept[-1]) > limit else kept[-1].decode("utf-8", "replace")
        line_start = kept_offsets[-1]
    else:
        text = b"\n".join(kept[cut:]).decode("utf-8", "replace")
        line_start = kept_offsets[cut]
    return text, count_lines(log, line_start) + 1


# ---------------------------------------------------------------------------------------------------------------------
# the two exit-status adapters (their records are the exit status: engine-made, never eligible for [[known]])


@register("exit")
def exit_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """Exit 0 -> one PASS record; anything else -> one FAIL record carrying the log's tail (the last 40 lines, at most
    2 KB, clipped from the end). The record is the exit status itself, so it is engine-made."""
    if status == 0 and not ctx.timed_out:
        return [Record(id=check.id, status="PASS", engine=True)]
    tail, line = log_tail(log)
    return [Record(id=check.id, status="FAIL", reason=f"exit {status}", assertion=tail or None,
                   log_line=line or None, engine=True)]


SKIP_LINE_RE = re.compile(r"^\s*SKIP:\s*(?P<reason>.*?)\s*$")


@register("exit77")
def exit77_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """The automake convention: exit 0 PASS; exit 77 SKIPPED with the last ``SKIP: <reason>`` line of the log as its
    reason (``exit_declared`` set); anything else (and a 77 whose log says no ``SKIP:``) FAIL with the tail."""
    if status == 0 and not ctx.timed_out:
        return [Record(id=check.id, status="PASS", engine=True)]
    if status == 77 and not ctx.timed_out:
        found: tuple[int, str] | None = None
        for no, text in read_lines(log):
            m = SKIP_LINE_RE.match(text)
            if m and m.group("reason"):
                found = (no, m.group("reason"))
        if found is not None:
            ctx.exit_declared = True
            return [Record(id=check.id, status="SKIPPED", reason=found[1], log_line=found[0], engine=True)]
        tail, line = log_tail(log)
        return [Record(id=check.id, status="FAIL", reason="exit 77 (skip) but the log gives no 'SKIP: <reason>' line",
                       assertion=tail or None, log_line=line or None, engine=True)]
    tail, line = log_tail(log)
    return [Record(id=check.id, status="FAIL", reason=f"exit {status}", assertion=tail or None,
                   log_line=line or None, engine=True)]


# ---------------------------------------------------------------------------------------------------------------------
# text helpers

ANSI_RE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b[@-Z\\-_]")
"""A terminal escape (colour, cursor) that a parser strips before matching."""


def clean_line(raw: bytes) -> str:
    """One log line as text: UTF-8 (bad bytes replaced), no line end, no ANSI escapes, the last ``\\r`` overwrite."""
    text = raw.decode("utf-8", "replace").rstrip("\r\n")
    text = ANSI_RE.sub("", text)
    if "\r" in text:
        last = text.rsplit("\r", 1)[-1]
        text = last if last.strip() else text.replace("\r", "")
    return text


def read_lines(log: Path) -> Iterator[tuple[int, str]]:
    """(1-based line number, cleaned text) of every line of ``log``; nothing for a missing log."""
    try:
        fh = log.open("rb")
    except (FileNotFoundError, IsADirectoryError):
        return
    with fh:
        for no, raw in enumerate(fh, 1):
            yield no, clean_line(raw)


_VOLATILE_RE = re.compile(r"\b(?:\d{4,}|[0-9a-f]{8,})\b")


def slug(text: str, limit: int = 60) -> str:
    """A stable id piece: lower case, ``[a-z0-9.]`` runs joined by ``-``, long numbers and hex as ``n``, at most
    ``limit`` characters (cut at a ``-``); ``x`` when nothing is left."""
    s = _VOLATILE_RE.sub("n", text.lower())
    s = re.sub(r"[^a-z0-9.]+", "-", s).strip("-.")
    if len(s) > limit:
        head = s[:limit]
        s = head.rsplit("-", 1)[0] if "-" in head else head
    return s or "x"


def desc_slug(text: str) -> str:
    """The slug of an ``ok:``/``FAIL:`` description: cut at ``  [`` (the HTTP bracket) and `` (`` (volatile detail)."""
    cut = text
    for mark in ("  [", " ("):
        index = cut.find(mark)
        if index > 0:
            cut = cut[:index]
    return slug(cut)


def phase_slug(name: str) -> str:
    """The id piece of a phase: its number (``4``, ``5a``, ``13b``) when it has one, else its slug."""
    m = re.match(r"^\s*(\d+[a-z]?)\b\.?", name)
    return m.group(1) if m else slug(name, 40)


def join_clip(lines: Iterable[str]) -> str | None:
    """Lines joined with newlines, trailing blank lines dropped, cut to the 2 KB of an assertion (None for none)."""
    out = list(lines)
    while out and not out[-1].strip():
        out.pop()
    while out and not out[0].strip():
        out.pop(0)
    return clip("\n".join(out)) if out else None


def tail_clip(lines: list[tuple[int, str]], limit: int = ASSERTION_MAX_BYTES) -> tuple[str | None, int | None]:
    """The last of ``lines`` within ``limit`` bytes (whole lines, from the end) and the line number of the first."""
    kept: list[tuple[int, str]] = []
    total = 0
    for no, text in reversed(lines):
        size = len(text.encode("utf-8", "replace")) + (1 if kept else 0)
        if total + size > limit:
            break
        kept.insert(0, (no, text))
        total += size
    if not kept and lines:
        no, text = lines[-1]
        raw = text.encode("utf-8", "replace")[-limit:]
        return raw.decode("utf-8", "ignore"), no
    while kept and not kept[0][1].strip():
        kept.pop(0)
    if not kept:
        return None, None
    return "\n".join(t for _, t in kept), kept[0][0]


class IdBook:
    """Hands out record ids, unique within one check: a repeat gets ``#2``, ``#3``."""

    def __init__(self) -> None:
        self._seen: dict[str, int] = {}

    def take(self, ident: str) -> str:
        n = self._seen.get(ident, 0) + 1
        self._seen[ident] = n
        return ident if n == 1 else f"{ident}#{n}"


# ---------------------------------------------------------------------------------------------------------------------
# who owns a stretch of a log


@dataclass
class Scope:
    """Who owns the records of one stretch of a log, and how they are named.

    - ``check``: the check that ran; ``unit_run``: it is a unit, so its records are its own (no attribution).
    - ``prefix``: the id prefix of a record no covered unit owns (the check id, or ``<gate>/<stage>``).
    - ``phase``: the stage or phase name (``Record.phase``); ``seams``: its seams from ``check.phases`` (None: the
      owner's).
    - ``prefer``: gate ids whose covered units win an ambiguous attribution (the running gate, a nested gate).
    - ``owner``: a covered unit that owns this whole stretch (a gate stage that runs exactly one unit), or None.
    """

    check: Check
    seam_map: SeamMap
    ids: IdBook
    prefix: str
    unit_run: bool
    phase: str | None = None
    seams: list[str] | None = None
    prefer: tuple[str, ...] = ()
    owner: Check | None = None

    def record(self, tail: str | None, status: str, *, unit: Check | None = None, **kw: object) -> Record:
        """A record named ``<unit>/<tail>`` when a covered unit (``unit`` or the scope's ``owner``) owns it, else
        ``<prefix>/<tail>``; ``tail`` None names the owner itself (``<unit>`` or ``<prefix>``)."""
        unit = unit or self.owner
        if unit is not None and unit.id != self.check.id:
            base, rec_check, seams = unit.id, unit.id, None
        else:
            base, rec_check, seams = self.prefix, None, self.seams
        ident = base if tail is None else f"{base}/{tail}"
        rec = Record(id=self.ids.take(ident), status=status, phase=self.phase, seams=seams, check=rec_check)
        for key, value in kw.items():
            setattr(rec, key, value)
        return rec

    def own(self, status: str, **kw: object) -> Record:
        """The scope's own verdict (``<prefix>``, never a covered unit's): a stage's PASS, FAIL or SKIPPED."""
        rec = Record(id=self.ids.take(self.prefix), status=status, phase=self.phase, seams=self.seams)
        for key, value in kw.items():
            setattr(rec, key, value)
        return rec


def phase_seams(check: Check, name: str) -> list[str] | None:
    """The seams ``check.phases`` maps a phase or stage name to (the longest matching prefix), or None."""
    best = ""
    for prefix in check.phases:
        if name.startswith(prefix) and len(prefix) > len(best):
            best = prefix
    return list(check.phases[best]) if best else None


def base_scope(check: Check, ctx: AdapterContext, ids: IdBook | None = None) -> Scope:
    """The scope of a whole log: records are ``<check>/…``; a gate prefers the units it covers."""
    return Scope(check=check, seam_map=ctx.seam_map, ids=ids or IdBook(), prefix=check.id,
                 unit_run=check.kind == "unit", prefer=(check.id,) if check.kind == "gate" else ())


# ---------------------------------------------------------------------------------------------------------------------
# reading a check's own scripts (stage lists, phase groups, which unit a stage runs)


def script_texts(check: Check, root: Path) -> list[str]:
    """The text of the scripts a check runs: every cmd element that names a file under the check's cwd, plus every
    inline ``sh -c`` program (an element with a newline or ``;``)."""
    out: list[str] = []
    base = root / check.cwd
    for part in check.cmd:
        if "\n" in part or ";" in part:
            out.append(part)
            continue
        if part.startswith("-") or "/" not in part:
            continue
        path = base / part
        try:
            if path.is_file() and path.stat().st_size < 4 * 1024 * 1024:
                out.append(path.read_text(encoding="utf-8", errors="replace"))
        except OSError:
            continue
    return out


_STAGE_ECHO_RE = re.compile(r"""(?:echo|printf)\s+(?:'[^']*'\s+)?["']== (?P<name>.+?) ==(?:\\n)?["']""")


def declared_stages(texts: Iterable[str]) -> list[tuple[str, str]]:
    """(stage name, the script text of that stage) for every ``echo "== name =="`` of the scripts, in order. A name
    with a shell expansion keeps the text before it (a prefix)."""
    out: list[tuple[str, str]] = []
    for text in texts:
        marks = list(_STAGE_ECHO_RE.finditer(text))
        for index, m in enumerate(marks):
            end = marks[index + 1].start() if index + 1 < len(marks) else len(text)
            name = m.group("name").split("$", 1)[0].rstrip()
            out.append((name, text[m.end():end]))
    return out


_FUNC_START_RE = re.compile(r"^(?P<name>[A-Za-z_][\w-]*)\s*\(\)\s*\{?\s*$")
_PHASE_CALL_RE = re.compile(r"""^\s*phase\s+(?P<q>["'])(?P<name>.+?)(?P=q)""")


def phase_groups(texts: Iterable[str]) -> list[list[str]]:
    """The phases each shell function of the scripts announces with ``phase "<name>"``, in order, one list per
    function that announces at least one (a name with a shell expansion keeps the text before it). These are the
    harness's inner runs (``inner``, ``inner3``): such a run that reached one of its phases must reach its last."""
    groups: list[list[str]] = []
    for text in texts:
        current: list[str] | None = None
        for line in text.splitlines():
            if current is None:
                m = _FUNC_START_RE.match(line)
                if m:
                    current = []
                continue
            if line.startswith("}"):
                if current:
                    groups.append(current)
                current = None
                continue
            m = _PHASE_CALL_RE.match(line)
            if m:
                current.append(m.group("name").split("$", 1)[0].rstrip())
    return groups


def _unit_script_keys(unit: Check) -> list[str]:
    """The words of a unit's cmd that a gate script running it would also contain (the script or dir it names, by its
    last two path parts)."""
    keys: list[str] = []
    for part in unit.cmd:
        if "/" in part and not part.startswith("-"):
            parts = [p for p in part.split("/") if p and p != ".."]
            keys.append("/".join(parts[-2:]))
    return keys


def stage_unit(check: Check, seam_map: SeamMap, stage_text: str, adapter: str, prefer: tuple[str, ...]) -> Check | None:
    """The one covered unit (``covered_by`` in ``prefer``, adapter ``adapter``) a gate stage's script runs, or None
    when none or several match."""
    found: list[Check] = []
    for unit in seam_map.checks:
        if unit.kind != "unit" or unit.covered_by not in prefer or unit.adapter != adapter:
            continue
        keys = _unit_script_keys(unit)
        if keys and all(key in stage_text for key in keys):
            found.append(unit)
    return found[0] if len(found) == 1 else None


# ---------------------------------------------------------------------------------------------------------------------
# libtest (cargo test)


_CARGO_VALUE_FLAGS = {"--manifest-path", "-p", "--package", "--test", "--bin", "--example", "--bench", "--features",
                      "-F", "--target", "--target-dir", "-j", "--jobs", "--profile", "--color", "--message-format",
                      "-Z", "--config", "--exclude"}


@dataclass(frozen=True)
class CargoUnit:
    """A libtest unit's test binary, read from its cmd: ``kind`` (``lib``/``test``/``bin``), ``name`` (the crate for
    ``lib``, the stem for ``test``, the bin name), its test name filters and whether they are ``--exact``."""

    check: Check
    kind: str
    name: str
    filters: tuple[str, ...] = ()
    exact: bool = False

    def matches(self, test: str) -> bool:
        if not self.filters:
            return True
        return any((test == f) if self.exact else (f in test) for f in self.filters)


def cargo_unit(unit: Check) -> CargoUnit | None:
    """The test binary a libtest unit's ``cargo test`` cmd runs, or None (not a ``cargo test`` cmd)."""
    cmd = list(unit.cmd)
    if len(cmd) < 2 or os.path.basename(cmd[0]) != "cargo" or "test" not in cmd:
        return None
    args = cmd[cmd.index("test") + 1:]
    package = kind = name = None
    filters: list[str] = []
    exact = False
    after_dashes = False
    i = 0
    while i < len(args):
        arg = args[i]
        if after_dashes:
            if arg == "--exact":
                exact = True
            elif not arg.startswith("-"):
                filters.append(arg)
        elif arg == "--":
            after_dashes = True
        elif arg in ("-p", "--package") and i + 1 < len(args):
            package = args[i + 1]
            i += 1
        elif arg == "--test" and i + 1 < len(args):
            kind, name = "test", args[i + 1]
            i += 1
        elif arg == "--bin" and i + 1 < len(args):
            kind, name = "bin", args[i + 1]
            i += 1
        elif arg == "--lib":
            kind = "lib"
        elif arg in _CARGO_VALUE_FLAGS:
            i += 1
        elif "=" in arg and arg.split("=", 1)[0] in _CARGO_VALUE_FLAGS:
            pass
        elif not arg.startswith("-"):
            filters.append(arg)
        i += 1
    if kind == "lib":
        if not package:
            return None
        name = package.replace("-", "_")
    if kind is None or name is None:
        return None
    return CargoUnit(unit, kind, name, tuple(filters), exact)


def cargo_units(seam_map: SeamMap) -> list[CargoUnit]:
    """Every libtest unit of the map with the binary it runs."""
    out: list[CargoUnit] = []
    for unit in seam_map.checks:
        if unit.kind == "unit" and unit.adapter == "libtest":
            cu = cargo_unit(unit)
            if cu is not None:
                out.append(cu)
    return out


_RUNNING_RE = re.compile(r"^\s*Running (?:(?P<ut>unittests) )?(?P<path>\S+)(?: \((?P<bin>[^)]*)\))?\s*$")
_DOC_RE = re.compile(r"^\s*Doc-tests (?P<crate>\S+)\s*$")
_RUN_N_RE = re.compile(r"^running (?P<n>\d+) tests?\s*$")
_TEST_RE = re.compile(r"^test (?P<name>.+?) \.\.\. ?(?P<rest>.*)$")
_QUIET_FAIL_RE = re.compile(r"^(?P<name>\S.*?) --- FAILED\s*$")
_BLOCK_RE = re.compile(r"^---- (?P<name>.+?) (?:stdout|stderr) ----\s*$")
_RESULT_RE = re.compile(r"^test result: (?P<res>ok|FAILED)\. (?P<passed>\d+) passed; (?P<failed>\d+) failed; "
                        r"(?P<ignored>\d+) ignored")
_CARGO_FAILED_RE = re.compile(r"(?:^|\s)error: test failed, to rerun pass (?P<what>.+)$")
_NOT_EXIT_RE = re.compile(r"process didn't exit successfully: .*\((?P<why>(?:signal|exit (?:status|code))[^)]*)\)\s*$")
_COMPILE_ERR_RE = re.compile(r"^error(?:\[E\d+\])?: ")
_COULD_NOT_RE = re.compile(r"^error: could not compile ")
_PANIC_RE = re.compile(r"^thread '[^']*'(?: \(\d+\))? panicked at ")
_PANIC_END_RE = re.compile(r"^(?:stack backtrace:|note: run with `RUST_BACKTRACE|note: Some details are omitted)")


def panic_text(lines: list[tuple[int, str]]) -> tuple[str | None, int | None]:
    """The assertion of a ``---- <test> stdout ----`` block: the ``panicked at`` line and its message (up to the
    backtrace or its note), or the block's whole text when nothing panicked; and its first line number."""
    for index, (no, text) in enumerate(lines):
        if _PANIC_RE.match(text):
            body = [text]
            for _no, more in lines[index + 1:]:
                if _PANIC_END_RE.match(more):
                    break
                body.append(more)
            return join_clip(body), no
    text = join_clip(t for _, t in lines)
    first = next((no for no, t in lines if t.strip()), lines[0][0] if lines else None)
    return text, first


@dataclass
class _Binary:
    """One test binary's results as the log shows them."""

    kind: str
    name: str
    label: str
    line: int
    candidates: list[CargoUnit]
    started: bool = False
    done: bool = False
    failed_count: int = 0
    results: dict[str, Record] = field(default_factory=dict)
    order: list[str] = field(default_factory=list)
    crashed: bool = False
    crash_why: str = ""
    crash_lines: list[tuple[int, str]] = field(default_factory=list)
    running_test: str | None = None
    running_line: int = 0
    tail: deque = field(default_factory=lambda: deque(maxlen=TAIL_LINES))


class Libtest:
    """The libtest parser (cargo test): fed one line at a time, ``finish`` returns the records."""

    def __init__(self, scope: Scope, units: list[CargoUnit] | None = None):
        self.scope = scope
        self.units = units if units is not None else cargo_units(scope.seam_map)
        self.binaries: list[_Binary] = []
        self.cur: _Binary | None = None
        self.block: tuple[str, list[tuple[int, str]]] | None = None
        self.blocks: dict[str, tuple[str | None, int | None]] = {}
        self.listing = False
        self.compile_err: list[tuple[int, str]] = []
        self.compile_open = False
        self.could_not: tuple[int, str] | None = None

    # -- attribution
    def _candidates(self, kind: str, name: str) -> list[CargoUnit]:
        if self.scope.unit_run or self.scope.owner is not None:
            return []
        found = [u for u in self.units if u.kind == kind and u.name == name.replace("-", "_")]
        preferred = [u for u in found if u.check.covered_by in self.scope.prefer]
        return preferred or found

    def _unit_for(self, binary: _Binary, test: str | None) -> Check | None:
        if self.scope.owner is not None:
            return self.scope.owner
        if self.scope.unit_run:
            return self.scope.check
        pool = binary.candidates
        if test is not None:
            pool = [u for u in pool if u.matches(test)]
            pool.sort(key=lambda u: -max((len(f) for f in u.filters), default=0))
        else:
            pool = [u for u in pool if not u.filters] or pool
        if not pool:
            return None
        if test is None and len({u.check.id for u in pool}) > 1:
            return None
        return pool[0].check

    # -- feeding
    def _open(self, kind: str, name: str, label: str, no: int) -> None:
        self._close_current()
        self.cur = _Binary(kind, name, label, no, self._candidates(kind, name))
        self.binaries.append(self.cur)

    def _close_current(self) -> None:
        self._end_block()
        cur = self.cur
        if cur is not None and cur.started and not cur.done:
            cur.crashed = True
        self.cur = None

    def _end_block(self) -> None:
        if self.block is not None:
            name, lines = self.block
            self.blocks[name] = panic_text(lines)
            self.block = None

    def _result(self, name: str, status: str, no: int, reason: str | None = None) -> None:
        cur = self.cur
        if cur is None:
            cur = self.cur = _Binary("tests", "tests", "tests", no, [])
            self.binaries.append(cur)
            cur.started = True
        if name not in cur.results:
            cur.order.append(name)
        cur.results[name] = Record(id="", status=status, reason=reason, log_line=no, test=name)

    def feed(self, no: int, text: str) -> None:
        if self.cur is not None:
            self.cur.tail.append((no, text))
        if self.compile_open:
            if text.strip():
                self.compile_err.append((no, text))
            else:
                self.compile_open = False
        m = _RUNNING_RE.match(text)
        if m and (m.group("bin") or m.group("path").endswith(".rs")):
            path = m.group("path")
            binname = os.path.basename(m.group("bin") or "")
            stem = re.sub(r"-[0-9a-f]{8,}$", "", binname)
            if m.group("ut"):
                kind = "lib" if path.endswith("lib.rs") else "bin"
                name = stem or Path(path).stem
                label = "lib" if kind == "lib" else f"bin-{name}"
            elif path.startswith("tests/") or "/tests/" in path:
                kind, name = "test", Path(path).stem
                label = name
            else:
                kind, name = "other", Path(path).stem
                label = slug(name)
            self._open(kind, name, label, no)
            return
        m = _DOC_RE.match(text)
        if m:
            self._open("doc", m.group("crate"), "doc", no)
            return
        if _COULD_NOT_RE.match(text) and self.could_not is None:
            self.could_not = (no, text)
        elif _COMPILE_ERR_RE.match(text) and not self.compile_err and not text.startswith("error: test failed") \
                and not text.startswith("error: could not compile") and not re.match(r"^error: \d+ targets? failed", text):
            self.compile_err.append((no, text))
            self.compile_open = True
        m = _RUN_N_RE.match(text)
        if m:
            if self.cur is None or self.cur.done:
                self._open("tests", "tests", "tests", no)
            assert self.cur is not None
            self.cur.started = True
            self.listing = False
            return
        m = _BLOCK_RE.match(text)
        if m:
            self._end_block()
            self.block = (m.group("name"), [])
            self.listing = False
            return
        m = _RESULT_RE.match(text)
        if m:
            self._end_block()
            self.listing = False
            if self.cur is not None:
                self.cur.done = True
                self.cur.failed_count = int(m.group("failed"))
            return
        if text.strip() == "failures:":
            if self.block is not None:
                self._end_block()
                self.listing = True
            return
        if self.block is not None:
            self.block[1].append((no, text))
            return
        if self.listing:
            m = re.match(r"^\s{4}(?P<name>\S.*?)\s*$", text)
            if m and self.cur is not None and m.group("name") not in self.cur.results:
                self._result(m.group("name"), "FAIL", no)
            return
        m = _TEST_RE.match(text)
        if m:
            name = re.sub(r" - should panic$", "", m.group("name"))
            rest = m.group("rest").strip()
            if rest == "ok":
                self._result(name, "PASS", no)
            elif rest == "FAILED":
                self._result(name, "FAIL", no)
            elif rest.startswith("ignored"):
                why = rest[len("ignored"):].lstrip(", ").strip()
                self._result(name, "SKIPPED", no, f"ignored: {why}" if why else "ignored")
            elif rest.startswith("bench:"):
                self._result(name, "PASS", no)
            elif self.cur is not None:
                self.cur.running_test = name
                self.cur.running_line = no
                if rest:
                    self._crash_line(no, rest)
            return
        m = _QUIET_FAIL_RE.match(text)
        if m and self.cur is not None:
            self._result(m.group("name"), "FAIL", no)
            return
        self._crash_line(no, text)

    def _crash_line(self, no: int, text: str) -> None:
        cur = self.cur
        if cur is None or cur.done or not cur.started:
            return
        if _CARGO_FAILED_RE.search(text):
            cur.crashed = True
            cur.crash_lines.append((no, text.strip()))
            return
        if cur.crashed:
            m = _NOT_EXIT_RE.search(text)
            if m:
                cur.crash_why = m.group("why")
                cur.crash_lines.append((no, text.strip()))

    # -- results
    def finish(self) -> list[Record]:
        self._close_current()
        out: list[Record] = []
        for binary in self.binaries:
            out.extend(self._binary_records(binary))
        if not self.binaries and (self.could_not or self.compile_err):
            lines = self.compile_err or ([self.could_not] if self.could_not else [])
            text, line = join_clip(t for _, t in lines), lines[0][0]
            out.append(self.scope.record("build", "FAIL", reason="cargo could not compile the tests",
                                         assertion=text, log_line=line))
        return out

    def _name(self, binary: _Binary, test: str | None) -> tuple[str | None, Check | None]:
        """The id tail and the owning unit of a test (or, with None, of the binary itself)."""
        unit = self._unit_for(binary, test)
        if unit is not None or self.scope.unit_run:
            return test, unit
        return (binary.label if test is None else f"{binary.label}/{test}"), None

    def _binary_records(self, binary: _Binary) -> list[Record]:
        out: list[Record] = []
        for name in binary.order:
            parsed = binary.results[name]
            tail, unit = self._name(binary, name)
            rec = self.scope.record(tail, parsed.status, unit=unit, reason=parsed.reason, log_line=parsed.log_line,
                                    test=name)
            if parsed.status == "FAIL" and name in self.blocks:
                text, line = self.blocks[name]
                rec.assertion = text
                rec.log_line = line or rec.log_line
            out.append(rec)
        if binary.crashed or (binary.started and not binary.done):
            why = binary.crash_why or "no 'test result' line"
            crash_text = join_clip(t for _, t in binary.crash_lines)
            running = binary.running_test
            if running is not None and running not in binary.results:
                tail, unit = self._name(binary, running)
                out.append(self.scope.record(
                    tail, "FAIL", unit=unit, test=running, reason=f"the test binary died during this test ({why})",
                    assertion=crash_text, log_line=binary.running_line or binary.line))
            else:
                tail, unit = self._name(binary, None)
                if crash_text is None:
                    crash_text, first = tail_clip(list(binary.tail))
                else:
                    first = binary.crash_lines[0][0]
                out.append(self.scope.record(tail, "FAIL", unit=unit,
                                             reason=f"the test binary did not finish ({why})",
                                             assertion=crash_text, log_line=first or binary.line))
        return out

    def accounts_for(self, status: int) -> bool:
        """True when this parse explains a failing exit: every binary finished, each one's ``test result`` failed
        count equals the FAIL records parsed for it, at least one, and the status is cargo's 101."""
        if status != 101 or not self.binaries:
            return False
        total = 0
        for binary in self.binaries:
            if binary.crashed or (binary.started and not binary.done):
                return False
            fails = sum(1 for r in binary.results.values() if r.status == "FAIL")
            if fails != binary.failed_count:
                return False
            total += fails
        return total > 0


@register("libtest")
def libtest_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """cargo test output: one record per test fn (module docstring); an ignored test is SKIPPED ``ignored``; a binary
    that did not finish is one FAIL; a compile failure is one FAIL ``<check>/build``."""
    parser = Libtest(base_scope(check, ctx))
    for no, text in read_lines(log):
        parser.feed(no, text)
    recs = parser.finish()
    ctx.exit_declared = parser.accounts_for(status)
    return recs


# ---------------------------------------------------------------------------------------------------------------------
# vitest


_VT_FILE_RE = re.compile(r"^\s?(?P<sym>[✓√×❯↓])\s+(?P<file>\S+\.(?:test|spec)\.[cm]?[jt]sx?)(?=\s|$)(?:\s+\((?P<counts>[^)]*)\))?")
_VT_TEST_RE = re.compile(r"^\s{3,}(?P<sym>[✓√×↓])\s+(?P<name>.+?)(?:\s+\d+(?:\.\d+)?m?s)?\s*$")
_VT_ARROW_RE = re.compile(r"^\s{3,}→\s?(?P<msg>.*)$")
_VT_FAIL_RE = re.compile(r"^\s*FAIL\s+(?P<file>\S+\.(?:test|spec)\.[cm]?[jt]sx?)\s+(?:>\s+(?P<names>.+)|\[ .* \])\s*$")
_VT_SEP_RE = re.compile(r"^\s*⎯{3,}")
_VT_TOTAL_RE = re.compile(r"^\s*(?P<what>Test Files|Tests)\s+(?P<counts>.*\(\d+\))\s*$")
_VT_TSC_RE = re.compile(r"^(?P<file>[^\s(:]+)(?:\((?P<l1>\d+),\d+\):|:(?P<l2>\d+):\d+ -) error (?P<code>TS\d+): (?P<msg>.*)$")
_VT_FILE_ARG_RE = re.compile(r"\S+\.(?:test|spec)\.[cm]?[jt]sx?$")


def vitest_units(seam_map: SeamMap) -> list[tuple[str, Check]]:
    """(test file as the unit's cmd names it, unit) for every vitest unit of the map."""
    out: list[tuple[str, Check]] = []
    for unit in seam_map.checks:
        if unit.kind == "unit" and unit.adapter == "vitest":
            for part in unit.cmd:
                if _VT_FILE_ARG_RE.search(part) and not part.startswith("-"):
                    out.append((part, unit))
    return out


def _same_file(a: str, b: str) -> bool:
    a, b = a.lstrip("./"), b.lstrip("./")
    return a == b or a.endswith("/" + b) or b.endswith("/" + a)


class Vitest:
    """The vitest parser: the default reporter's text (``pnpm check``, and the log of a unit run), and the JSON
    reporter's file when a unit wrote one (``load_json``)."""

    def __init__(self, scope: Scope, base_dir: Path):
        self.scope = scope
        self.base_dir = base_dir
        self.units = [] if scope.unit_run else vitest_units(scope.seam_map)
        self.recs: dict[tuple[str, str | None], Record] = {}
        self.file: str | None = None
        self.last_test: str | None = None
        self.block: tuple[str, str | None, int, list[str]] | None = None
        self.fail_lines: dict[tuple[str, str | None], int] = {}
        self.failed_total: int | None = None
        self.typecheck: list[Record] = []

    def rel(self, file: str) -> str:
        """A test file as vitest prints it (relative to the UI dir), also for the JSON reporter's absolute names."""
        try:
            return str(Path(file).relative_to(self.base_dir))
        except ValueError:
            return file

    def owner(self, file: str) -> Check | None:
        if self.scope.unit_run:
            return self.scope.check
        for unit_file, unit in self.units:
            if _same_file(file, unit_file):
                return unit
        return None

    def make(self, file: str, names: str | None, status: str, **kw: object) -> Record:
        unit = self.owner(file)
        if unit is not None:
            tail = names
        else:
            tail = file if names is None else f"{file}/{names}"
        return self.scope.record(tail, status, unit=unit, test=names, **kw)

    def put(self, file: str, names: str | None, status: str, *, override: bool = False, **kw: object) -> None:
        key = (file, names)
        rec = self.recs.get(key)
        if rec is None:
            self.recs[key] = self.make(file, names, status, **kw)
            return
        if status == "FAIL" or rec.status != "FAIL":
            rec.status = status
        for k, v in kw.items():
            if v is not None and (override or getattr(rec, k) is None):
                setattr(rec, k, v)

    def _end_block(self) -> None:
        if self.block is None:
            return
        file, names, no, lines = self.block
        body: list[str] = []
        for text in lines:
            if re.match(r"^\s*\d+\|", text) or re.match(r"^\s*\|", text):
                continue
            body.append(text)
            if re.match(r"^\s*❯ \S+:\d+:\d+", text):
                break
        reason = None if names else "the test file failed to load"
        self.put(file, names, "FAIL", override=True, reason=reason, assertion=join_clip(body), log_line=no)
        self.block = None

    def feed(self, no: int, text: str) -> None:
        if self.block is not None:
            if _VT_SEP_RE.match(text) or _VT_TOTAL_RE.match(text) or _VT_FAIL_RE.match(text):
                self._end_block()
            else:
                self.block[3].append(text)
                return
        m = _VT_FAIL_RE.match(text)
        if m:
            key = (m.group("file"), m.group("names"))
            self.fail_lines.setdefault(key, no)
            self.block = (m.group("file"), m.group("names"), no, [])
            return
        m = _VT_TOTAL_RE.match(text)
        if m:
            if m.group("what") == "Tests":
                f = re.search(r"(\d+) failed", m.group("counts"))
                self.failed_total = int(f.group(1)) if f else 0
            return
        m = _VT_TSC_RE.match(text)
        if m:
            line = m.group("l1") or m.group("l2")
            self.typecheck.append(self.scope.record(f"typecheck/{m.group('file')}:{line}/{m.group('code')}", "FAIL",
                                                    reason="typecheck", assertion=clip(text), log_line=no))
            return
        m = _VT_FILE_RE.match(text)
        if m:
            self.file = m.group("file")
            sym, counts = m.group("sym"), m.group("counts") or ""
            if sym in "✓√" or (sym == "❯" and "failed" not in counts):
                self.put(self.file, None, "PASS", log_line=no)
            elif sym == "↓":
                self.put(self.file, None, "SKIPPED", reason="skipped", log_line=no)
            self.last_test = None
            return
        m = _VT_TEST_RE.match(text)
        if m and self.file is not None:
            name, sym = m.group("name"), m.group("sym")
            status = {"✓": "PASS", "√": "PASS", "×": "FAIL", "↓": "SKIPPED"}[sym]
            self.put(self.file, name, status, reason="skipped" if status == "SKIPPED" else None, log_line=no)
            self.last_test = name if status == "FAIL" else None
            return
        m = _VT_ARROW_RE.match(text)
        if m and self.file is not None and self.last_test is not None:
            self.put(self.file, self.last_test, "FAIL", assertion=clip(m.group("msg")) or None)

    def finish(self) -> list[Record]:
        self._end_block()
        with_tests = {file for (file, names) in self.recs if names is not None}
        out = [rec for (file, names), rec in self.recs.items()
               if not (names is None and rec.status == "PASS" and file in with_tests)]
        return out + self.typecheck

    def fail_line(self, file: str, names: str | None) -> int | None:
        """The log line of the text reporter's ``FAIL <file> > <names>`` block for a JSON result, or None."""
        for (known_file, known_names), no in self.fail_lines.items():
            if known_names == names and _same_file(file, known_file):
                return no
        return None

    def load_json(self, path: Path) -> list[Record] | None:
        """Records from the JSON reporter's file (named afresh: the text parse's ids are not consumed), or None
        when it is absent or unreadable."""
        try:
            data = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            return None
        if not isinstance(data, dict) or not isinstance(data.get("testResults"), list):
            return None
        self.scope = replace(self.scope, ids=IdBook())
        out: list[Record] = []
        for suite in data["testResults"]:
            if not isinstance(suite, dict):
                continue
            file = self.rel(str(suite.get("name", "?")))
            asserts = suite.get("assertionResults") or []
            if suite.get("status") == "failed" and not asserts:
                out.append(self.make(file, None, "FAIL", reason="the test file failed to load",
                                     assertion=clip(str(suite.get("message") or "")) or None,
                                     log_line=self.fail_line(file, None)))
                continue
            for item in asserts:
                if not isinstance(item, dict):
                    continue
                names = " > ".join([*map(str, item.get("ancestorTitles") or []), str(item.get("title", "?"))])
                st = item.get("status")
                status, reason = {"passed": ("PASS", None), "failed": ("FAIL", None),
                                  "todo": ("SKIPPED", "todo")}.get(st, ("SKIPPED", "skipped"))  # type: ignore[arg-type]
                assertion = None
                if status == "FAIL":
                    msgs = [str(msg) for msg in item.get("failureMessages") or []]
                    assertion = join_clip(ln for msg in msgs for ln in msg.splitlines() if not re.match(r"^\s+at ", ln))
                line = self.fail_line(file, names) if status == "FAIL" else None
                out.append(self.make(file, names, status, reason=reason, assertion=assertion, log_line=line))
        return out


def vitest_json_path(check: Check, ctx: AdapterContext) -> Path:
    """Where the JSON reporter wrote: the cmd's ``--outputFile`` (tokens expanded, relative to the check's cwd), else
    ``<scratch>/vitest.json``."""
    tokens = {"root": str(ctx.root), "scratch": str(ctx.scratch), "tmp": str(ctx.tmp)}
    cmd = list(check.cmd)
    for index, part in enumerate(cmd):
        value = None
        if part.startswith("--outputFile="):
            value = part.split("=", 1)[1]
        elif part == "--outputFile" and index + 1 < len(cmd):
            value = cmd[index + 1]
        elif part.startswith("--outputFile.json="):
            value = part.split("=", 1)[1]
        if value:
            for key, val in tokens.items():
                value = value.replace("{" + key + "}", val)
            path = Path(value)
            return path if path.is_absolute() else ctx.root / check.cwd / path
    return ctx.scratch / "vitest.json"


@register("vitest")
def vitest_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """vitest: the JSON reporter's file when it was written (per-test records), else the default reporter's text
    (``✓``/``×``/``↓`` lines, the ``FAIL <file> > <names>`` blocks, typecheck errors of ``pnpm check``)."""
    parser = Vitest(base_scope(check, ctx), ctx.root / check.cwd)
    for no, text in read_lines(log):
        parser.feed(no, text)
    text_recs = parser.finish()
    json_recs = parser.load_json(vitest_json_path(check, ctx)) if check.kind == "unit" else None
    recs = json_recs + parser.typecheck if json_recs else text_recs
    fails = sum(1 for r in recs if r.status == "FAIL")
    loads = sum(1 for r in recs if r.reason == "the test file failed to load")
    ctx.exit_declared = (status == 1 and fails > 0 and parser.failed_total is not None
                         and fails == parser.failed_total + len(parser.typecheck) + loads)
    return recs


# ---------------------------------------------------------------------------------------------------------------------
# pytest / unittest


_UT_VERBOSE_RE = re.compile(r"^(?P<name>\w+) \((?P<cls>[\w.]+)\)(?: \[.*\])? \.\.\. (?P<rest>.*)$")
_UT_HEAD_RE = re.compile(r"^(?P<kind>FAIL|ERROR): (?P<name>\w+) \((?P<cls>[\w.]+)\)")
_UT_RAN_RE = re.compile(r"^Ran (?P<n>\d+) tests? in ")
_UT_END_RE = re.compile(r"^(?P<res>OK|FAILED|NO TESTS RAN)(?: \((?P<extra>.*)\))?\s*$")
_PY_FAILED_RE = re.compile(r"^(?P<kind>FAILED|ERROR) (?P<node>\S+?)(?: - (?P<msg>.*))?$")
_PY_SKIPPED_RE = re.compile(r"^SKIPPED \[\d+\] (?P<where>\S+?):? (?P<msg>.*)$")
_PY_SUMMARY_RE = re.compile(r"^=*\s*(?P<body>(?:\d+ (?:failed|passed|skipped|errors?|xfailed|xpassed|deselected|"
                            r"warnings?)(?:, )?)+) in [\d.]+s(?: \([^)]*\))?\s*=*$")
_PY_SECTION_RE = re.compile(r"^_{3,} (?P<name>.+?) _{3,}$")


def _ut_name(name: str, cls: str) -> str:
    return cls if cls.endswith("." + name) else f"{cls}.{name}"


def exception_text(lines: list[str]) -> str | None:
    """The exception of a Python traceback block: from the first unindented line after the last ``Traceback``."""
    start = 0
    for index, text in enumerate(lines):
        if text.startswith("Traceback (most recent call last):"):
            start = index + 1
    for index in range(start, len(lines)):
        text = lines[index]
        if text and not text[0].isspace():
            return join_clip(lines[index:])
    return join_clip(lines)


class PyTests:
    """The pytest / unittest text parser (either output; several runs in one log are fine)."""

    def __init__(self, scope: Scope):
        self.scope = scope
        self.recs: list[Record] = []
        self.by_key: dict[str, Record] = {}
        self.block: tuple[str, int, list[str]] | None = None
        self.block_kind = ""
        self.ran: int | None = None
        self.declared_fails = 0
        self.ended = 0
        self.skip_counts: list[int] = []
        self.verbose_skips = 0
        self.ok_results = 0
        self.py_section: tuple[str, int, list[str]] | None = None
        self.py_blocks: dict[str, tuple[str | None, int]] = {}

    def put(self, key: str, status: str, **kw: object) -> Record:
        rec = self.by_key.get(key)
        if rec is None:
            rec = self.scope.record(key, status, test=key, **kw)
            self.by_key[key] = rec
            self.recs.append(rec)
        else:
            if status == "FAIL":
                rec.status = "FAIL"
            for k, v in kw.items():
                if v is not None:
                    setattr(rec, k, v)
        return rec

    def _end_block(self) -> None:
        if self.block is None:
            return
        key, no, lines = self.block
        body = lines
        if body and re.match(r"^-{20,}$", body[0]):
            body = body[1:]
        reason = "error" if self.block_kind == "ERROR" else None
        self.put(key, "FAIL", assertion=exception_text(body), log_line=no, reason=reason)
        self.block = None

    def feed(self, no: int, text: str) -> None:
        if self.py_section is not None:
            if _PY_SECTION_RE.match(text) or text.startswith("====="):
                name, start, lines = self.py_section
                errs = [ln for ln in lines if ln.startswith("E ")]
                loc = [ln for ln in lines if re.match(r"^\S+:\d+: ", ln)]
                self.py_blocks[name] = (join_clip(errs + loc[-1:]) or join_clip(lines), start)
                self.py_section = None
            else:
                self.py_section[2].append(text)
                return
        if self.block is not None:
            if re.match(r"^={20,}$", text) or (re.match(r"^-{20,}$", text) and self.block[2]):
                self._end_block()
                if re.match(r"^-{20,}$", text):
                    return
            else:
                self.block[2].append(text)
                return
        m = _UT_HEAD_RE.match(text)
        if m:
            self.block_kind = m.group("kind")
            self.block = (_ut_name(m.group("name"), m.group("cls")), no, [])
            return
        m = _UT_VERBOSE_RE.match(text)
        if m:
            key = _ut_name(m.group("name"), m.group("cls"))
            rest = m.group("rest").strip()
            if rest == "ok" or rest == "expected failure":
                self.put(key, "PASS", log_line=no)
                self.ok_results += 1
            elif rest in ("FAIL", "ERROR", "unexpected success"):
                self.put(key, "FAIL", log_line=no, reason="unexpected success" if rest == "unexpected success" else None)
            elif rest.startswith("skipped"):
                why = rest[len("skipped"):].strip().strip("'\"")
                self.put(key, "SKIPPED", log_line=no, reason=f"skipped: {why}" if why else "skipped")
                self.verbose_skips += 1
            return
        m = _UT_RAN_RE.match(text)
        if m:
            self.ran = int(m.group("n"))
            return
        m = _UT_END_RE.match(text)
        if m and self.ran is not None:
            extra = m.group("extra") or ""
            counts = {k: int(v) for k, v in re.findall(r"(\w[\w ]*?)=(\d+)", extra)}
            self.declared_fails += counts.get("failures", 0) + counts.get("errors", 0) + \
                counts.get("unexpected successes", 0)
            self._close_run(no, m.group("res"), counts.get("skipped", 0))
            return
        m = _PY_SECTION_RE.match(text)
        if m:
            self.py_section = (m.group("name"), no, [])
            return
        m = _PY_FAILED_RE.match(text)
        if m and "::" in m.group("node"):
            node = m.group("node")
            short = node.rsplit("::", 1)[-1]
            text_block, line = self.py_blocks.get(short, self.py_blocks.get(node, (None, no)))
            msg = m.group("msg")
            self.put(node, "FAIL", assertion=text_block or (clip(msg) if msg else None), log_line=line,
                     reason="error" if m.group("kind") == "ERROR" else None)
            return
        m = _PY_SKIPPED_RE.match(text)
        if m:
            self.put(m.group("where"), "SKIPPED", reason=f"skipped: {m.group('msg')}", log_line=no)
            self.verbose_skips += 1
            return
        m = _PY_SUMMARY_RE.match(text)
        if m:
            body = m.group("body")
            counts = {k.rstrip("s") if k.startswith("error") else k: int(v)
                      for v, k in re.findall(r"(\d+) (\w+)", body)}
            self.declared_fails += counts.get("failed", 0) + counts.get("error", 0)
            self.ran = sum(counts.get(k, 0) for k in ("failed", "passed", "skipped", "error"))
            self._close_run(no, "FAILED" if counts.get("failed") or counts.get("error") else "OK",
                            counts.get("skipped", 0))

    def _close_run(self, no: int, res: str, skipped: int) -> None:
        self._end_block()
        self.ended += 1
        if res == "NO TESTS RAN" or self.ran == 0:
            self.recs.append(self.scope.record(None if self.scope.unit_run or self.scope.owner else "run", "FAIL",
                                               reason="no tests ran", log_line=no))
        elif res == "OK" and not self.ok_results:
            self.recs.append(self.scope.record(None if self.scope.unit_run or self.scope.owner else "run", "PASS",
                                               reason=None, log_line=no))
        if skipped and skipped > self.verbose_skips:
            self.recs.append(self.scope.record("skipped", "SKIPPED", log_line=no,
                                               reason=f"{skipped - self.verbose_skips} test(s) skipped (named only "
                                                      f"with -v / -rs)"))
        self.verbose_skips = 0
        self.ok_results = 0
        self.ran = None

    def finish(self) -> list[Record]:
        self._end_block()
        return list(self.recs)

    def accounts_for(self, status: int) -> bool:
        fails = sum(1 for r in self.recs if r.status == "FAIL")
        return status == 1 and self.ended > 0 and fails > 0 and fails == self.declared_fails


def _py_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    parser = PyTests(base_scope(check, ctx))
    for no, text in read_lines(log):
        parser.feed(no, text)
    recs = parser.finish()
    if parser.ended == 0 and status == 0 and not ctx.timed_out:
        tail, line = log_tail(log)
        recs.append(Record(id=check.id, status="FAIL", reason="no test summary ('Ran N tests' / pytest's '… in Ns') "
                           "was printed", assertion=tail or None, log_line=line or None))
    ctx.exit_declared = parser.accounts_for(status)
    return recs


register("pytest")(_py_adapter)
register("unittest")(_py_adapter)


# ---------------------------------------------------------------------------------------------------------------------
# okfail (every sh harness) and the emitter file


_OK_RE = re.compile(r"^\s*(?:(?P<who>[\w.-]+): )?(?P<kind>ok|FAIL|note): (?P<text>.*?)\s*$")
_PHASE_RE = re.compile(r"^== (?P<name>.+?)(?:\s+\(t\+[\d.]+s\))?(?: ==)?\s*$")
_SKIPPED_CLOSE_RE = re.compile(r"^(?P<gate>[A-Z][\w-]*) SKIPPED: (?P<reason>.+?)\s*$")
_GREEN_CLOSE_RE = re.compile(r"^(?:[\w.-]+: )?(?:[A-Z][\w-]*(?: [\w-]+)*? )?(?:ALL (?:CASES )?)?GREEN\b|"
                             r"^(?:[\w.-]+: )?all ok\s*$|^[\w .-]+: PASS\b")
_RED_CLOSE_RE = re.compile(r"^(?P<gate>[\w.-]+(?: [\w.-]+)*?): (?P<n>\d+) FAIL\s*$|"
                           r"^(?P<gate2>[A-Z][\w-]*) RED\b(?:.*?(?P<n2>\d+) FAIL)?|"
                           r"^(?:[\w.-]+: )?(?P<n3>\d+) (?:case\(s\) failed|failure\(s\))\s*$|^[\w .-]+: FAIL\s*$")
_KEPT_RE = re.compile(r"scratch kept at (?P<dir>\S+)")
_REQUIRE_FAIL_RE = re.compile(r"^[\w-]+: .*\(REQUIRE_[\w]+=1\)$")


def harness_script_groups(check: Check, root: Path) -> list[list[str]]:
    """``phase_groups`` of the scripts the check runs (``[]`` when none can be read)."""
    return phase_groups(script_texts(check, root))


EMIT_STATUSES = {"ok": "PASS", "PASS": "PASS", "FAIL": "FAIL", "SKIPPED": "SKIPPED", "skip": "SKIPPED"}
"""The ``status`` words a ``T111_RESULTS`` line may carry, and the record status each becomes."""


def read_emitter(path: Path, scope: Scope, lines_of: dict[str, int] | None = None) -> list[Record] | None:
    """The records of a ``T111_RESULTS`` file (X6, F3 phase 2), or None when it is absent or holds no line.

    One JSON object per line: ``status`` (``ok``/``PASS``, ``FAIL``, ``SKIPPED``), ``text`` (the description, the
    words after ``ok:``/``FAIL:``), and optionally ``phase``, ``id`` (an explicit ``@id``: ``<check>/<id>``) and
    ``reason`` (a SKIPPED's). The id is ``<check>/<phase>/<slug of text>`` otherwise. A line that is not such an object
    is one FAIL ``<check>/emitter`` naming it (a broken emitter never reads as green). ``lines_of`` maps a description
    to the log line that printed it (the record's ``log_line``)."""
    try:
        raw = path.read_text(encoding="utf-8", errors="replace")
    except (OSError, IsADirectoryError):
        return None
    lines = [ln for ln in raw.splitlines() if ln.strip()]
    if not lines:
        return None
    out: list[Record] = []
    for index, line in enumerate(lines, 1):
        try:
            obj = json.loads(line)
        except ValueError:
            obj = None
        status = EMIT_STATUSES.get(str(obj.get("status"))) if isinstance(obj, dict) else None
        if not isinstance(obj, dict) or status is None:
            out.append(scope.record("emitter", "FAIL", reason=f"T111_RESULTS line {index} is not a result object",
                                    assertion=clip(line)))
            continue
        text = str(obj.get("text") or obj.get("desc") or "")
        phase = str(obj.get("phase") or "") or None
        if obj.get("id"):
            tail = str(obj["id"])
        else:
            tail = (f"{phase_slug(phase)}/" if phase else "") + desc_slug(text)
        rec = scope.record(tail, status, reason=obj.get("reason") or (("skipped" if status == "SKIPPED" else None)),
                           assertion=clip(text) if status == "FAIL" else None,
                           log_line=(lines_of or {}).get(text))
        rec.phase = phase
        if phase:
            rec.seams = phase_seams(scope.check, phase) or rec.seams
        out.append(rec)
    return out


class OkFail:
    """The ``ok:`` / ``FAIL:`` / ``note:`` parser with ``== <phase>`` headers. ``strict`` adds the closing-line
    rules of a whole harness (a stage of a gate is not strict)."""

    def __init__(self, scope: Scope, *, strict: bool = True):
        self.scope = scope
        self.strict = strict
        self.recs: list[Record] = []
        self.phase: str | None = None
        self.phases_seen: list[str] = []
        self.fails = 0
        self.oks = 0
        self.closing: tuple[str, int, str] | None = None  # (kind green|red|skipped, line no, text)
        self.kept: str | None = None
        self.skip_reason: str | None = None
        self.red_count: int | None = None
        self.last_fail: tuple[int, str] | None = None
        self.lines_of: dict[str, int] = {}

    def feed(self, no: int, text: str) -> None:
        m = _PHASE_RE.match(text)
        if m:
            self.phase = m.group("name")
            self.phases_seen.append(self.phase)
            self.closing = None  # a closing line counts only as the harness's last word
            return
        m = _OK_RE.match(text)
        if m:
            kind, desc = m.group("kind"), m.group("text")
            if kind != "note":
                self.lines_of.setdefault(desc, no)
            if kind == "note":
                k = _KEPT_RE.search(desc)
                if k:
                    self.kept = k.group("dir")
                return
            self.closing = None
            seams = phase_seams(self.scope.check, self.phase) if self.phase else None
            tail = (f"{phase_slug(self.phase)}/" if self.phase else "") + desc_slug(desc)
            if kind == "ok":
                self.oks += 1
                rec = self.scope.record(tail, "PASS", log_line=no)
            else:
                self.fails += 1
                rec = self.scope.record(tail, "FAIL", assertion=clip(desc), log_line=no)
                self.last_fail = (no, text)
                if self.strict and _REQUIRE_FAIL_RE.match(desc):
                    self.closing = ("red", no, text)  # skip_or_fail under REQUIRE_<G>=1: its last line
                    self.red_count = None
            rec.phase = self.phase or self.scope.phase
            if seams is not None and rec.check is None:
                rec.seams = seams
            self.recs.append(rec)
            return
        if not self.strict:
            return
        m = _SKIPPED_CLOSE_RE.match(text)
        if m:
            self.closing = ("skipped", no, text)
            self.skip_reason = m.group("reason")
            return
        m = _RED_CLOSE_RE.match(text)
        if m:
            self.closing = ("red", no, text)
            n = m.group("n") or m.group("n2") or m.group("n3")
            self.red_count = int(n) if n else None
            return
        if _GREEN_CLOSE_RE.match(text):
            self.closing = ("green", no, text)

    def finish(self, groups: list[list[str]] | None = None,
               tail: tuple[str | None, int | None] = (None, None)) -> list[Record]:
        """The records, plus (strict) the harness verdict: ``tail`` is the log's last lines, the evidence of a
        harness that never closed; ``groups`` its inner runs (``phase_groups``)."""
        recs = list(self.recs)
        if self.kept:
            for rec in recs:
                rec.kept = self.kept
        if not self.strict:
            return recs
        last = self.phase or "none"
        closing = self.closing

        def verdict(status: str, reason: str, assertion: str | None = None, line: int | None = None) -> Record:
            rec = self.scope.record(None, status, reason=reason, assertion=assertion, log_line=line)
            rec.kept = self.kept
            return rec

        if closing is None:
            recs.append(verdict("FAIL", f"harness did not finish (last phase: {last})", *tail))
            return recs
        kind, line, text = closing
        if kind == "skipped":
            if self.fails:
                recs.append(verdict("FAIL", f"SKIPPED after {self.fails} FAIL line(s): {self.skip_reason}", text,
                                    line))
            else:
                recs.append(verdict("SKIPPED", self.skip_reason or "skipped", None, line))
            return recs
        unclosed = self._unclosed_run(groups or [])
        if unclosed is not None:
            recs.append(verdict("FAIL", f"harness did not finish (last phase: {unclosed})", text, line))
            return recs
        if kind == "red" and not self.fails:
            recs.append(verdict("FAIL", "the harness reported failures but printed no FAIL: line", text, line))
        elif kind == "green" and not self.recs:
            recs.append(verdict("PASS", None, None, line))  # type: ignore[arg-type]
        return recs

    def _unclosed_run(self, groups: list[list[str]]) -> str | None:
        """The last phase of an inner run that reached one of its phases but not its last, or None."""
        for group in groups:
            reached = [p for p in self.phases_seen if any(p.startswith(g) for g in group)]
            if not reached:
                continue
            if not any(p.startswith(group[-1]) for p in self.phases_seen):
                return reached[-1]
        return None

    def accounts_for(self, status: int) -> bool:
        return (status == 1 and self.fails > 0 and self.closing is not None and self.closing[0] == "red"
                and self.red_count == self.fails)


@register("okfail")
def okfail_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """A sh harness: ``ok:`` PASS, ``FAIL:`` FAIL (the text up to ``  [HTTP`` slugs the id, the whole text is the
    assertion), ``== <phase>`` headers mapped to seams by ``check.phases``, the closing line judged (module docstring
    of options.md §3.4): a missing closing line or an inner run whose last phase never came is FAIL ``harness did not
    finish (last phase: <name>)``; a ``<G> SKIPPED: <reason>`` after ``FAIL:`` lines is FAIL; ``note: scratch kept at
    <dir>`` fills ``kept``. A ``T111_RESULTS`` file with lines replaces the ``ok:``/``FAIL:`` parse (the closing line
    is still judged from the log)."""
    scope = base_scope(check, ctx)
    parser = OkFail(scope)
    tail: deque = deque(maxlen=TAIL_LINES)
    for no, text in read_lines(log):
        parser.feed(no, text)
        tail.append((no, text))
    recs = parser.finish(harness_script_groups(check, ctx.root), tail_clip(list(tail)))
    emitted = read_emitter(ctx.results, replace(scope, ids=IdBook()), parser.lines_of) if ctx.results else None
    if emitted:
        verdicts = [r for r in recs if r.id == check.id]
        recs = emitted + verdicts
    ctx.exit_declared = parser.accounts_for(status)
    return recs


@register("emitter")
def emitter_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """The shared emitter's file (F3 phase 2) when it holds lines, judged with the log's closing line; the ``okfail``
    parse of the log otherwise (the fallback while a harness is not instrumented)."""
    return okfail_adapter(check, log, status, ctx)


# ---------------------------------------------------------------------------------------------------------------------
# table (the Monocypher harness) and buildpy (0-knowledge/_meta/build.py --check)


_TABLE_HEAD_RE = re.compile(r"^FILE\s+VECTORS\s+CHECKS\s+PASS\s+FAIL\s+SKIP\s+RESULT\s*$")
_TABLE_ROW_RE = re.compile(r"^(?P<file>\S.*?)\s+(?P<vectors>\d+)\s+(?P<checks>\d+)\s+(?P<pass>\d+)\s+(?P<fail>\d+)\s+"
                           r"(?P<skip>\d+)\s+(?P<result>PASS|FAIL)\s*$")
_TABLE_FAIL_RE = re.compile(r"^\s*FAIL \[(?P<file>[^\]]+)\] (?P<msg>.*)$")


@register("table")
def table_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """The Monocypher harness: one record per row of its ``FILE … RESULT`` table (``n/a`` rows are inventory, not
    results), the ``FAIL [<file>] <msg>`` lines of that file as the assertion; no table is FAIL ``the harness printed
    no result table``."""
    scope = base_scope(check, ctx)
    fails: dict[str, list[tuple[int, str]]] = {}
    rows: list[tuple[int, re.Match[str]]] = []
    in_table = False
    tail: deque = deque(maxlen=TAIL_LINES)
    for no, text in read_lines(log):
        tail.append((no, text))
        m = _TABLE_FAIL_RE.match(text)
        if m:
            fails.setdefault(m.group("file"), []).append((no, m.group(0).strip()))
            continue
        if _TABLE_HEAD_RE.match(text):
            in_table = True
            continue
        if in_table:
            m = _TABLE_ROW_RE.match(text)
            if m:
                rows.append((no, m))
            elif text.strip() and " n/a -- " not in text:
                in_table = False
    recs: list[Record] = []
    for no, m in rows:
        name = m.group("file")
        if name == "TOTAL":
            continue
        key = name.split(" (")[0] if name.endswith(")") and not name.startswith("(") else name
        if m.group("result") == "PASS" and int(m.group("fail")) == 0:
            recs.append(scope.record(slug(name, 80), "PASS", log_line=no))
        else:
            lines = fails.get(key) or fails.get(name) or []
            recs.append(scope.record(slug(name, 80), "FAIL", reason=f"{m.group('fail')} of {m.group('checks')} "
                                     f"checks failed", assertion=join_clip(t for _, t in lines) or clip(m.group(0)),
                                     log_line=lines[0][0] if lines else no))
    if not rows:
        text, line = tail_clip(list(tail))
        recs.append(Record(id=check.id, status="FAIL", reason="the harness printed no result table", assertion=text,
                           log_line=line))
    ctx.exit_declared = status != 0 and any(r.status == "FAIL" for r in recs) and bool(rows)
    return recs


_BP_FILE_RE = re.compile(r"^(?P<path>\S.*):\s*$")
_BP_FIND_RE = re.compile(r"^  (?P<level>ERROR|WARN)(?::(?P<line>\d+))?  (?P<msg>.*)$")
_BP_TOTAL_RE = re.compile(r"^(?P<errors>\d+) error\(s\), (?P<warns>\d+) warning\(s\)\s*$")


@register("buildpy")
def buildpy_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """``0-knowledge/_meta/build.py --check``: one FAIL per ``ERROR[:<line>]  <msg>`` under its ``<path>:`` heading
    (id ``<check>/<path>:<line>/<code>``, the file and line in the assertion); WARN lines are not results; no
    ``N error(s), M warning(s)`` line is FAIL ``build.py did not finish``; a clean run is one PASS."""
    scope = base_scope(check, ctx)
    path = "?"
    recs: list[Record] = []
    total: int | None = None
    tail: deque = deque(maxlen=TAIL_LINES)
    for no, text in read_lines(log):
        tail.append((no, text))
        m = _BP_FIND_RE.match(text)
        if m:
            if m.group("level") == "ERROR":
                line = m.group("line")
                where = f"{path}:{line}" if line else path
                code_m = re.match(r"^([A-Z]\d{2}):", m.group("msg"))
                code = code_m.group(1) if code_m else desc_slug(m.group("msg"))
                recs.append(scope.record(f"{where}/{code}", "FAIL", assertion=clip(f"{where}: {m.group('msg')}"),
                                         log_line=no, reason="knowledge-base error"))
            continue
        m = _BP_TOTAL_RE.match(text)
        if m:
            total = int(m.group("errors"))
            continue
        m = _BP_FILE_RE.match(text)
        if m and not text.startswith(" "):
            path = m.group("path")
    if total is None:
        text, line = tail_clip(list(tail))
        recs.append(Record(id=check.id, status="FAIL", reason="build.py did not finish (no 'N error(s), M warning(s)' "
                           "line)", assertion=text, log_line=line))
    elif not recs and total == 0:
        recs.append(Record(id=check.id, status="PASS"))
    elif total != sum(1 for r in recs if r.status == "FAIL"):
        recs.append(Record(id=check.id, status="FAIL", reason=f"build.py counted {total} error(s), the log shows "
                           f"{sum(1 for r in recs if r.status == 'FAIL')}"))
    ctx.exit_declared = status == 1 and total is not None and total > 0 and total == len(recs)
    return recs


# ---------------------------------------------------------------------------------------------------------------------
# stagewise (check.sh and the mesh111 gate)


_STAGE_RE = re.compile(r"^== (?P<name>.+?) ==\s*$")
_STAGE_CLOSE_RE = re.compile(r"^(?:ALL GREEN|GREEN \(.*\).*|check\.sh: (?:all green|green\b.*))\s*$")
_NESTED_CLOSE_RE = re.compile(r"^check\.sh: (?:all green|green\b.*)\s*$")
_STAGE_SKIP_RE = re.compile(r"^(?:!! )?(?P<what>.*?)\s*—\s*(?P<name>[^—]*?)\s*SKIPPED\b(?P<rest>.*)$")

STAGE_INNER: tuple[tuple[re.Pattern[str], str], ...] = (
    (re.compile(r"^(?:test\b|G2\b)"), "libtest"),
    (re.compile(r"^(?:notices generator tests|presence sidecar)\b"), "pytest"),
    (re.compile(r"^(?:gate assertions|build scripts)\b"), "okfail"),
    (re.compile(r"^ui\b"), "vitest"),
    (re.compile(r"^messaging\b"), "nested"),
    (re.compile(r"^$"), "libtest"),
)
"""Stage name -> the inner adapter its lines go to (the first match; none: the stage is judged by its position).
``nested`` runs another gate's stages (``messaging`` = ``5-messaging/scripts/check.sh``) until that gate's closing
line; the text before the first header is the stage ``""`` (``3-intelligence/scripts/check.sh`` has no headers)."""

NESTED_GATES: dict[str, str] = {"messaging": "5-messaging/scripts/check.sh"}
"""Stage name prefix -> the script of the gate it nests (a gate whose cmd names it); that gate's covered units win
its tests' attribution."""


def inner_kind(name: str) -> str | None:
    """The inner adapter of a stage name (``STAGE_INNER``; the stage's own name, a nested one without its parent),
    or None."""
    for pattern, kind in STAGE_INNER:
        if pattern.match(name):
            return kind
    return None


@dataclass
class _Stage:
    """One stage seen in a gate's log: its name, its path (``messaging/test`` when nested), where it starts, its
    scope and inner parser, its last lines, and a skip line that named it."""

    name: str
    path: str
    line: int
    scope: Scope
    parser: Libtest | Vitest | PyTests | OkFail | None
    tail: deque = field(default_factory=lambda: deque(maxlen=TAIL_LINES))
    skipped: str | None = None
    skip_line: int | None = None


def _gate_of_script(seam_map: SeamMap, script: str) -> str | None:
    """The gate whose cmd runs ``script`` (a repo-relative path), or None."""
    for check in seam_map.checks:
        if check.kind == "gate" and any(script in part for part in check.cmd):
            return check.id
    return None


def stage_slug(*names: str) -> str:
    """The id piece of a stage (and of the stage it is nested in): each name's slug, at most 32 characters
    (``clippy (host)`` -> ``clippy-host``; ``messaging (mesh111 workspace)``, ``test`` -> ``messaging-mesh111-workspace/test``)."""
    return "/".join(slug(name, 32) for name in names if name)


@register("stagewise")
def stagewise_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """A gate of ``== name ==`` stages (options.md §3.4): each stage's lines go to its inner adapter
    (``STAGE_INNER``); a ``… — <what> SKIPPED`` line is a SKIPPED record (``<gate>/<what>``, or the open stage when it
    names nothing); a stage that ran with no inner result is one PASS ``<gate>/<stage>``; on a non-zero exit the open
    stage FAILs with its last 40 lines (unless its inner adapter parsed a FAIL there) and the stages the gate's
    script declares after it are SKIPPED ``not reached (stopped at <stage>)``."""
    ids = IdBook()
    declared = declared_stages(script_texts(check, ctx.root))
    bodies = dict(declared)
    units = cargo_units(ctx.seam_map)
    skips: list[Record] = []

    def make(name: str, path: str, no: int, prefer: tuple[str, ...], body: str, parent: str = "") -> _Stage:
        kind = inner_kind(name)
        scope = Scope(check=check, seam_map=ctx.seam_map, ids=ids,
                      prefix=f"{check.id}/{stage_slug(parent, name)}" if path else check.id, unit_run=False,
                      phase=path or None, seams=phase_seams(check, path) or phase_seams(check, name), prefer=prefer)
        parser: Libtest | Vitest | PyTests | OkFail | None = None
        if kind == "libtest":
            parser = Libtest(scope, units)
        elif kind == "vitest":
            parser = Vitest(scope, ctx.root / check.cwd)
        elif kind in ("pytest", "okfail"):
            scope.owner = stage_unit(check, ctx.seam_map, body, kind, prefer)
            parser = PyTests(scope) if kind == "pytest" else OkFail(scope, strict=False)
        return _Stage(name, path, no, scope, parser)

    current = make("", "", 0, (check.id,), "")
    stages = [current]
    top: _Stage | None = None
    nested_gate: str | None = None  # set while a nesting stage runs another gate's stages
    for no, text in read_lines(log):
        m = _STAGE_RE.match(text)
        if m:
            name = m.group("name")
            if top is not None and nested_gate is not None:
                current = make(name, f"{top.name}/{name}", no, (nested_gate, check.id), "", parent=top.name)
            else:
                current = top = make(name, name, no, (check.id,), bodies.get(name, ""))
                if inner_kind(name) == "nested":
                    script = next((sc for prefix, sc in NESTED_GATES.items() if name.startswith(prefix)), None)
                    nested_gate = (_gate_of_script(ctx.seam_map, script) if script else None) or check.id
            stages.append(current)
            continue
        if nested_gate is not None and _NESTED_CLOSE_RE.match(text):
            nested_gate = None
            current = top  # type: ignore[assignment]
            continue
        current.tail.append((no, text))
        if not _STAGE_CLOSE_RE.match(text):
            m = _STAGE_SKIP_RE.match(text)
            if m:
                named = m.group("name").strip()
                reason = (text[3:] if text.startswith("!! ") else text).strip()
                if named:
                    nest = top.name if top is not None and current is not top else ""
                    where = f"{nest}/{named}" if nest else named
                    skips.append(Record(id=ids.take(f"{check.id}/{stage_slug(nest, named)}"), status="SKIPPED",
                                        reason=reason, log_line=no, phase=where,
                                        seams=phase_seams(check, where) or phase_seams(check, named)))
                else:
                    current.skipped, current.skip_line = reason, no
                continue
        if current.parser is not None:
            current.parser.feed(no, text)
    recs: list[Record] = []
    declared_exit = False
    for stage in stages:
        inner = stage.parser.finish() if stage.parser is not None else []
        recs.extend(inner)
        if status != 0 and stage is current:
            if any(r.status == "FAIL" for r in inner):
                declared_exit = isinstance(stage.parser, (Libtest, PyTests, OkFail)) and \
                    stage.parser.accounts_for(status)
                if isinstance(stage.parser, Vitest):
                    declared_exit = False
                continue
            text, line = tail_clip(list(stage.tail))
            where = stage.path or "(before the first stage)"
            recs.append(stage.scope.own("FAIL", reason=f"exit {status} in stage {where}", assertion=text,
                                        log_line=line or stage.line or None))
        elif stage.skipped is not None:
            recs.append(stage.scope.own("SKIPPED", reason=stage.skipped, log_line=stage.skip_line))
        elif not inner and stage.path and inner_kind(stage.name) != "nested":
            recs.append(stage.scope.own("PASS", log_line=stage.line))
    recs.extend(skips)
    if status != 0:
        seen = [s.name for s in stages if s.path and "/" not in s.path]
        reached = [i for i, (name, _body) in enumerate(declared) if any(s.startswith(name) for s in seen)]
        stopped = current.path or "(before the first stage)"
        for name, _body in declared[(max(reached) + 1) if reached else 0:]:
            if any(s.startswith(name) for s in seen):
                continue
            recs.append(Record(id=ids.take(f"{check.id}/{stage_slug(name)}"), status="SKIPPED",
                               reason=f"not reached (stopped at {stopped})", phase=name,
                               seams=phase_seams(check, name)))
    ctx.exit_declared = declared_exit
    return recs
