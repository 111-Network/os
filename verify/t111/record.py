"""The result records of ``<scratch>/<run-id>/results.jsonl`` (options.md §3.3).

The file starts with one ``run`` record (head, dirty flag, euid, tools found, namespaces usable), then one ``check``
record per result, one JSON object per line. ``status`` is only PASS, FAIL or SKIPPED; a SKIPPED record always has a
``reason``.

An adapter (``t111.adapters``) returns bare ``Record`` objects (id, status, reason, assertion, log_line, maybe phase,
seams, after, rerun, test). ``enrich`` fills the rest from the map: the layers, the spec, the two sides, ``read_first``,
the fake, the strength, the tier, the rerun command and its cost. ``mark_after`` adds the cascade link, ``apply_known``
the ``[[known]]`` rule (F7) and ``apply_require`` the ``--require`` rule.

``read_first`` follows one rule: a check against a fake (strength F with a ``[[fake]]`` on the seam) points at the side
under test, the one the fake does not mirror; every other failure points at the seam's ``read_first`` (the producer of
the data the assertion reads, which is also the producer of a contract payload).
"""

from __future__ import annotations

import fnmatch
import json
import shlex
import threading
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Iterable, Mapping

from .schema import Check, Known, SeamMap

STATUSES: tuple[str, ...] = ("PASS", "FAIL", "SKIPPED")
"""The only record statuses."""

ASSERTION_MAX_BYTES = 2048
"""``assertion`` keeps at most 2 KB of the failing line(s), cut on a UTF-8 boundary."""

RECORD_VERSION = 1
"""``v`` of every record."""


@dataclass
class Record:
    """One ``check`` record. Adapters set the first block; ``enrich`` and the engine set the rest.

    - ``id``: ``<check>`` or ``<check>/<phase>/<slug>`` or ``<check>/<test path>``; ``hygiene/<check>`` for a leftover.
    - ``status``: PASS | FAIL | SKIPPED.
    - ``reason``: why it was SKIPPED (always set then), or a short cause of a FAIL (``timeout after 5 s ...``).
    - ``assertion``: the failing assertion's own words, verbatim, at most 2 KB.
    - ``log_line``: the 1-based line in the check's ``out.log`` where the evidence starts.
    - ``phase``: the harness phase the result came from, when the adapter knows it.
    - ``seams``: set by an adapter when a phase maps to other seams than the check's; else the check's seams.
    - ``after``: the id of the first FAIL of the same check when this FAIL follows it (read that one first).
    - ``rerun``: set by an adapter for a single-test command; else rendered from the check's template.
    - ``test``: the test name an adapter parsed (fills ``{test}`` in the rerun template; not written).
    - ``check``: the catalog check the record belongs to; an adapter of a gate may set a covered unit's id here.
    """

    id: str
    status: str
    reason: str | None = None
    assertion: str | None = None
    log_line: int | None = None
    phase: str | None = None
    seams: list[str] | None = None
    after: str | None = None
    rerun: str | None = None
    test: str | None = None
    check: str | None = None
    # filled by enrich() / the engine
    layers: list[Any] = field(default_factory=list)
    log: str | None = None
    rerun_cost: str | None = None
    spec: list[str] = field(default_factory=list)
    sides: dict[str, Any] = field(default_factory=dict)
    read_first: str | None = None
    fake: dict[str, str] | None = None
    strength: str | None = None
    tier: str | None = None
    duration_s: float | None = None

    def to_json(self) -> dict[str, Any]:
        """The JSON object of the record, keys in the order of options.md §3.3."""
        return {
            "v": RECORD_VERSION,
            "kind": "check",
            "id": self.id,
            "check": self.check,
            "seams": list(self.seams or []),
            "layers": list(self.layers),
            "status": self.status,
            "reason": self.reason,
            "assertion": self.assertion,
            "log": self.log,
            "log_line": self.log_line,
            "rerun": self.rerun,
            "rerun_cost": self.rerun_cost,
            "spec": list(self.spec),
            "sides": self.sides,
            "read_first": self.read_first,
            "fake": self.fake,
            "strength": self.strength,
            "tier": self.tier,
            "duration_s": self.duration_s,
            "phase": self.phase,
            "after": self.after,
        }


def clip(text: str | None, limit: int = ASSERTION_MAX_BYTES) -> str | None:
    """Cut ``text`` to ``limit`` UTF-8 bytes without splitting a character."""
    if text is None:
        return None
    raw = text.encode("utf-8", "replace")
    if len(raw) <= limit:
        return text
    return raw[:limit].decode("utf-8", "ignore")


def render(template: str, values: Mapping[str, str | None]) -> str:
    """Replace each ``{name}`` of ``values`` in ``template``; unknown or unset placeholders stay as they are."""
    out = template
    for key, value in values.items():
        if value is not None:
            out = out.replace("{" + key + "}", value)
    return out


def default_rerun(check: Check) -> str:
    """The rerun command when the check has no template: the cmd itself, from its cwd."""
    cmd = " ".join(shlex.quote(part) for part in check.cmd)
    return cmd if check.cwd in ("", ".") else f"(cd {shlex.quote(check.cwd)} && {cmd})"


def enrich(rec: Record, check: Check, seam_map: SeamMap, *, root: Path | None = None) -> Record:
    """Fill every map-derived field of ``rec`` from ``check`` (or the unit an adapter named in ``rec.check``)."""
    owner = check
    if rec.check and rec.check != check.id:
        unit = seam_map.check(rec.check)
        if unit is not None:
            owner = unit
    rec.check = owner.id
    if rec.seams is None:
        rec.seams = list(owner.seams)
    rec.tier = rec.tier or owner.tier
    rec.strength = rec.strength or owner.strength
    if rec.rerun is None:
        template = owner.rerun or default_rerun(owner)
        rec.rerun = render(template, {"id": rec.id, "test": rec.test, "check": owner.id,
                                      "root": str(root) if root else None})
    if rec.rerun_cost is None:
        rec.rerun_cost = owner.rerun_cost or owner.tier
    layers: list[Any] = list(owner.layers)
    spec: list[str] = []
    primary = None
    for seam_id in rec.seams:
        seam = seam_map.seam(seam_id)
        if seam is None:
            continue
        primary = primary or seam
        for layer in seam.layers():
            if layer not in layers:
                layers.append(layer)
        for ref in seam.spec:
            if ref not in spec:
                spec.append(ref)
    if not rec.layers:
        rec.layers = layers
    if not rec.spec:
        rec.spec = spec
    if primary is not None:
        rec.sides = {"a": primary.a.to_json(), "b": primary.b.to_json()}
        rec.read_first = primary.read_first
        if owner.strength == "F":
            fakes = seam_map.fakes_on(primary.id)
            if fakes:
                fake = fakes[0]
                rec.fake = {"path": fake.path, "mirrors": fake.mirrors}
                rec.read_first = "b" if fake.mirrors == "a" else "a"
    return rec


def mark_after(records: Iterable[Record]) -> None:
    """Give every FAIL after the first FAIL of the same check ``after = <first FAIL id>`` (unless an adapter set it)."""
    first: dict[str, str] = {}
    for rec in records:
        if rec.status != "FAIL" or rec.id.startswith("hygiene/"):
            continue
        key = rec.check or ""
        if key in first:
            if rec.after is None:
                rec.after = first[key]
        else:
            first[key] = rec.id


def known_condition_holds(condition: str, facts: Mapping[str, bool]) -> bool:
    """Evaluate a ``known.condition`` against the run's facts (``{"euid0": bool}``)."""
    euid0 = bool(facts.get("euid0", False))
    if condition == "euid0":
        return euid0
    if condition == "nonroot":
        return not euid0
    return False


def apply_known(rec: Record, known: Iterable[Known], facts: Mapping[str, bool]) -> Record:
    """The F7 rule: a matching FAIL (``fails``) or PASS (``vacuous``) under a holding condition becomes SKIPPED."""
    for entry in known:
        if entry.check and entry.check != rec.check:
            continue
        if not fnmatch.fnmatchcase(rec.id, entry.test):
            continue
        if not known_condition_holds(entry.condition, facts):
            continue
        target = "FAIL" if entry.kind == "fails" else "PASS"
        if rec.status == target:
            if rec.status == "FAIL" and rec.assertion is None and rec.reason:
                rec.assertion = rec.reason
            rec.status = "SKIPPED"
            rec.reason = f"known: {entry.words()} ({entry.source})"
        break
    return rec


def apply_require(rec: Record) -> Record:
    """``--require``: a SKIPPED record becomes FAIL, its reason kept."""
    if rec.status == "SKIPPED":
        rec.status = "FAIL"
        rec.reason = f"required, was SKIPPED: {rec.reason}"
    return rec


class ResultsWriter:
    """Appends JSON lines to ``results.jsonl``, one write and flush per record, safe across worker threads."""

    def __init__(self, path: Path):
        self.path = path
        self._lock = threading.Lock()
        self._fh = path.open("a", encoding="utf-8")

    def write(self, obj: Mapping[str, Any]) -> None:
        line = json.dumps(obj, ensure_ascii=False, sort_keys=False) + "\n"
        with self._lock:
            self._fh.write(line)
            self._fh.flush()

    def close(self) -> None:
        with self._lock:
            self._fh.close()


def read_results(path: Path) -> list[dict[str, Any]]:
    """Read a ``results.jsonl`` back (the ``run`` record first); for ``--matrix``, ``--list`` estimates and tests."""
    out: list[dict[str, Any]] = []
    with path.open(encoding="utf-8") as fh:
        for line in fh:
            line = line.strip()
            if line:
                out.append(json.loads(line))
    return out
