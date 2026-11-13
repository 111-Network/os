"""The human summary (options.md §3.3): ``summary.txt``, also printed on stdout, and the run's exit code.

The form::

    verify: 1214 PASS, 2 FAIL, 9 SKIPPED — 14 checks, fast+slow, 6m12s — DIR/<run-id>/results.jsonl
    FAIL L3-L4:chat-relay [L3↔L4] l1.node111/api_v1_relay/<test fn>: <assertion, first line>
         read first: <files of the side to read> (a, under test; b is a fake) · spec 4-interface/README.md §10
         rerun: cargo test … -- --exact <test fn> · log: …/l1.gate/out.log:5121
    SKIPPED L2-L7:epoch-linkkey l7.g11: T0 stub (7-relay/TASKS.md V5)
    VERIFY RED (2 FAIL)

One line of counts (then, when the map has fakes checked against nothing, one ``note:`` line with their count, the
reminder §3.5 asks every run to repeat, and one ``note:`` line per dead run whose leftovers the start-of-run sweep
killed), one block per FAIL, one line per SKIPPED, and the last line:
``VERIFY GREEN`` (exit 0), ``VERIFY GREEN (n SKIPPED — not a full run)`` (exit 0) or ``VERIFY RED (n FAIL)`` (exit 1).
``--require`` has already turned every SKIPPED into FAIL before this module counts. A usage error exits 2
(``EXIT_USAGE``).
"""

from __future__ import annotations

import re
from typing import Iterable

from .record import Record, safe_text

ERROR_LIKE = re.compile(r"(?i)(\berr(or)?\b|\bfail|\bpanic|\bassert|exception|traceback|\bfatal\b|\babort|\bnot ok\b|"
                        r"\bdenied\b|\brefused\b|\btimed? ?out\b|\bunexpected\b|\bmismatch|✗|✘)")
"""A line that looks like an error (the headline prefers the last such line of the assertion)."""

EXIT_GREEN = 0
EXIT_RED = 1
EXIT_USAGE = 2


def duration(seconds: float) -> str:
    """``6m12s`` or ``42s`` (whole seconds)."""
    total = int(round(seconds))
    return f"{total // 60}m{total % 60:02d}s" if total >= 60 else f"{total}s"


def layer_tag(layers: Iterable[object]) -> str:
    """``[L3↔L4]`` from the record's layers (``X`` for the cross-cutting pseudo layer)."""
    names: list[str] = []
    ints = sorted({layer for layer in layers if isinstance(layer, int)})
    names.extend(f"L{n}" for n in ints)
    if any(layer == "X" for layer in layers):
        names.append("X")
    return "[" + ("↔".join(names) if names else "-") + "]"


def key_line(text: str | None) -> str | None:
    """The line of ``text`` a headline quotes: the last non-empty line that looks like an error, else the last
    non-empty line (None for no text)."""
    lines = [line.strip() for line in (text or "").splitlines() if line.strip()]
    if not lines:
        return None
    errors = [line for line in lines if ERROR_LIKE.search(line)]
    return errors[-1] if errors else lines[-1]


def headline(rec: Record) -> str:
    """The one line a FAIL block opens with: the reason, then the assertion's key line (``key_line``)."""
    line = key_line(rec.assertion)
    if rec.reason:
        return f"{rec.reason}: {line}" if line and line != rec.reason else rec.reason
    return line or "(no assertion text)"


def fail_block(rec: Record) -> list[str]:
    """The lines of one FAIL."""
    seam = rec.seams[0] if rec.seams else "-"
    out = [f"FAIL {seam} {layer_tag(rec.layers)} {rec.id}: {headline(rec)}"]
    if rec.after:
        out.append(f"     after: {rec.after} (read that failure first)")
    if rec.sides and rec.read_first in ("a", "b"):
        side = rec.sides.get(rec.read_first, {})
        files = ", ".join(side.get("files", [])) or "(no files listed)"
        if rec.fake:
            other = "a" if rec.read_first == "b" else "b"
            how = f"{rec.read_first}, under test; {other} is a fake"
        else:
            how = rec.read_first
        line = f"     read first: {files} ({how})"
        if rec.spec:
            line += " · spec " + ", ".join(rec.spec)
        out.append(line)
    tail = []
    if rec.rerun:
        tail.append(f"rerun: {rec.rerun}")
    if rec.log:
        tail.append(f"log: {rec.log}" + (f":{rec.log_line}" if rec.log_line else ""))
    if tail:
        out.append("     " + " · ".join(tail))
    if rec.kept:
        out.append(f"     kept by the harness: {rec.kept}")
    return out


def last_line(n_fail: int, n_skipped: int) -> tuple[str, int]:
    """The closing line and the exit code."""
    if n_fail:
        return f"VERIFY RED ({n_fail} FAIL)", EXIT_RED
    if n_skipped:
        return f"VERIFY GREEN ({n_skipped} SKIPPED — not a full run)", EXIT_GREEN
    return "VERIFY GREEN", EXIT_GREEN


def render(records: list[Record], *, n_checks: int, tiers: Iterable[str], duration_s: float, results: str,
           parity_none: int = 0, notes: Iterable[str] = ()) -> tuple[str, int]:
    """The summary text (ending in a newline; ``safe_text``, so printing it never raises) and the exit code.
    ``notes`` are extra ``note: ...`` lines after the counts."""
    n_pass = sum(1 for r in records if r.status == "PASS")
    n_fail = sum(1 for r in records if r.status == "FAIL")
    n_skip = sum(1 for r in records if r.status == "SKIPPED")
    lines = [f"verify: {n_pass} PASS, {n_fail} FAIL, {n_skip} SKIPPED — {n_checks} checks, {'+'.join(tiers)}, "
             f"{duration(duration_s)} — {results}"]
    if parity_none:
        lines.append(f"note: {parity_none} fake(s) checked against nothing they mirror (parity = \"none\")")
    lines.extend(notes)
    for rec in records:
        if rec.status == "FAIL":
            lines.extend(fail_block(rec))
    for rec in records:
        if rec.status == "SKIPPED":
            seam = rec.seams[0] if rec.seams else "-"
            lines.append(f"SKIPPED {seam} {rec.id}: {rec.reason}")
    closing, code = last_line(n_fail, n_skip)
    lines.append(closing)
    return safe_text("\n".join(lines) + "\n"), code
