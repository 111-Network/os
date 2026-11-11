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
The engine fills everything else from the map. Two engine rules hold whatever an adapter returns: a timeout always
yields a FAIL record with the reason ``timeout ...``, and a non-zero exit with no FAIL and no SKIPPED record yields a
FAIL ``exit <n> but no failing result was parsed`` (a lying parse never turns red into green).
"""

from __future__ import annotations

from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable

from .record import Record, clip
from .schema import Check, SeamMap

TAIL_LINES = 40
"""How many closing log lines a FAIL record of the ``exit`` adapter carries."""


@dataclass(frozen=True)
class AdapterContext:
    """What an adapter may read besides the log.

    - ``root``: the repo root; ``scratch``: the check's scratch dir; ``tmp``: its ``TMPDIR``.
    - ``results``: the ``T111_RESULTS`` file the check may have written (X6 emitter), maybe absent.
    - ``timed_out``: the check hit ``timeout_s`` and was killed.
    - ``seam_map``: the loaded map (for ``check.phases`` and covered units).
    - ``facts``: run facts (``euid0``, ``offline``) an adapter may need for a skip reason.
    """

    root: Path
    scratch: Path
    tmp: Path
    results: Path
    timed_out: bool
    seam_map: SeamMap
    facts: dict[str, bool] = field(default_factory=dict)


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


def log_tail(log: Path, lines: int = TAIL_LINES) -> tuple[str, int]:
    """The last ``lines`` lines of ``log`` and the 1-based line number where they start."""
    try:
        text = log.read_text(encoding="utf-8", errors="replace")
    except FileNotFoundError:
        return "", 0
    all_lines = text.splitlines()
    start = max(0, len(all_lines) - lines)
    return "\n".join(all_lines[start:]), start + 1 if all_lines else 0


@register("exit")
def exit_adapter(check: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
    """Exit 0 -> one PASS record; anything else -> one FAIL record carrying the last 40 log lines."""
    if status == 0 and not ctx.timed_out:
        return [Record(id=check.id, status="PASS")]
    tail, line = log_tail(log)
    return [Record(id=check.id, status="FAIL", reason=f"exit {status}", assertion=clip(tail) or None,
                   log_line=line or None)]
