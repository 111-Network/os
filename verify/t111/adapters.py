"""The adapter registry: how a check's log becomes result records (options.md §3.4).

Row X2 owns only this registry, its contract and the trivial ``exit`` adapter. Row X3 owns the rest of this file: the
``libtest``, ``vitest``, ``pytest``/``unittest``, ``okfail``, ``stagewise``, ``table``, ``buildpy`` and ``emitter``
adapters, each registered with ``@register("<name>")``.

The contract. An adapter is ``fn(check, log, status, ctx) -> list[Record]``:

- ``check``: the ``t111.schema.Check`` that ran.
- ``log``: the path of the check's combined, ordered log (stdout and stderr in one stream, ``out.log``).
- ``status``: the exit status (negative = killed by that signal, as ``subprocess`` reports it).
- ``ctx``: an ``AdapterContext`` (below).

It returns bare ``t111.record.Record`` objects: ``id`` (``<check>`` or ``<check>/<phase>/<slug>``), ``status``, and
whatever it can read: ``reason``, ``assertion`` (verbatim), ``log_line``, ``phase``, ``seams`` (from ``check.phases``),
``after``, ``test``, ``rerun``, and ``check`` (a covered unit's id, for records a gate's output yields for that unit).
The engine fills everything else from the map. The engine's rules hold whatever an adapter returns:

- a timeout always yields a FAIL record with the reason ``timeout ...``;
- a non-zero exit with no FAIL record always yields the FAIL ``exit <n> but no failing result was parsed`` (a lying
  parse never turns red into green), PASS and SKIPPED records notwithstanding, unless the adapter sets
  ``ctx.exit_declared = True``. An adapter sets it only when its records explain this exact exit status: a harness
  whose documented skip code is 77 and whose log says why it skipped, say. Never for "some tests were ignored";
- a check that could not start, or that the runner killed on an interrupt, yields one FAIL with the check's id;
- ``[[known]]`` (F7) applies only to the records an adapter parsed from a test's own output: every record the engine
  makes (the rules above, skips before the start, lock waits, hygiene) is never eligible. An adapter never sets
  ``Record.engine`` (the engine's mark of its own records);
- the engine keeps one record per check id: a rule above replaces the adapter's record of the check's own id.

``log_tail`` is the helper for a tail: it reads from the end of the log (never the whole file into memory), keeps the
last lines within the 2 KB of an ``assertion`` (clipped from the end on a line boundary) and returns the 1-based line
where the kept text starts.
"""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from .record import ASSERTION_MAX_BYTES, Record
from .schema import Check, SeamMap

TAIL_LINES = 40
"""How many closing log lines a FAIL record of the ``exit`` adapter carries."""


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


@register("exit")
def exit_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """Exit 0 -> one PASS record; anything else -> one FAIL record carrying the log's tail (the last 40 lines, at most
    2 KB, clipped from the end)."""
    if status == 0 and not ctx.timed_out:
        return [Record(id=check.id, status="PASS")]
    tail, line = log_tail(log)
    return [Record(id=check.id, status="FAIL", reason=f"exit {status}", assertion=tail or None,
                   log_line=line or None)]
