"""Which checks a command line selects, and in which order (options.md §3.1).

Two shapes of run:

- **Full** (no ``--seam``/``--check``; ``--layer`` and ``--tier`` narrow it): every gate and unit that matches. A unit
  whose covering gate (``covered_by``) is in the plan is dropped; one whose gate matches the tiers but not the layers
  is replaced by that gate (the gate's adapter yields the unit's records, so no suite runs twice); one whose gate is
  outside the tiers runs alone. ``--gate`` keeps gates only, and puts a selected unit's covering gate in its place.
- **Targeted** (``--seam`` and/or ``--check``): ``--check`` runs exactly the named checks (any tier unless ``--tier``
  is given); ``--seam`` runs the units that cover the seams (their ``seams`` or ``phases``), and a gate only where no
  unit covers a seam, within the tiers (default ``fast,slow``). With ``--gate`` each unit is replaced by its gate.

Then, in every shape: each selected check pulls in its ``requires`` (transitively), and the plan is ordered fast, then
slow, then ns, catalog order within a tier, with every prerequisite before the checks that require it.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .schema import DEFAULT_TIERS, TIERS, Check, SeamMap


class SelectionError(Exception):
    """The selection names an unknown layer, seam, check or tier, or selects nothing (a usage error: exit 2)."""


@dataclass(frozen=True)
class Selection:
    """What the command line asked for (empty = no filter)."""

    layers: tuple[str, ...] = ()
    seams: tuple[str, ...] = ()
    checks: tuple[str, ...] = ()
    tiers: tuple[str, ...] = ()
    gate: bool = False

    @property
    def targeted(self) -> bool:
        """True for a ``--seam``/``--check`` run."""
        return bool(self.seams or self.checks)


@dataclass
class Plan:
    """The checks to run in order; ``pulled`` names prerequisites added by ``requires``; ``notes`` explain choices
    (for ``--list``), ``omitted`` names checks that cover the selection but sit in a tier not asked for."""

    checks: list[Check]
    tiers: tuple[str, ...]
    pulled: set[str] = field(default_factory=set)
    notes: list[str] = field(default_factory=list)
    omitted: list[str] = field(default_factory=list)


def _layer_key(layer: object) -> str:
    return str(layer)


def _covers_seam(check: Check, seam_id: str) -> bool:
    if seam_id in check.seams:
        return True
    return any(seam_id in seams for seams in check.phases.values())


def _validate(seam_map: SeamMap, sel: Selection) -> None:
    for tier in sel.tiers:
        if tier not in TIERS:
            raise SelectionError(f"unknown tier {tier!r} (one of {', '.join(TIERS)})")
    known_layers = {_layer_key(layer) for c in seam_map.checks for layer in c.layers}
    for layer in sel.layers:
        if layer not in known_layers and layer not in {str(n) for n in range(10)} | {"X"}:
            raise SelectionError(f"unknown layer {layer!r} (0..9 or X)")
    for seam_id in sel.seams:
        if seam_map.seam(seam_id) is None:
            raise SelectionError(f"unknown seam {seam_id!r}")
    for check_id in sel.checks:
        if seam_map.check(check_id) is None:
            raise SelectionError(f"unknown check {check_id!r}")


def select(seam_map: SeamMap, sel: Selection) -> Plan:
    """Turn a ``Selection`` into a ``Plan`` (raises ``SelectionError``)."""
    _validate(seam_map, sel)
    by_id = {c.id: c for c in seam_map.checks}
    explicit_tiers = bool(sel.tiers)
    tiers = tuple(sel.tiers) if sel.tiers else DEFAULT_TIERS
    in_tier = (lambda c: c.tier in tiers)
    in_layer = (lambda c: not sel.layers or any(_layer_key(layer) in sel.layers for layer in c.layers))
    chosen: list[str] = []
    notes: list[str] = []
    omitted: list[str] = []

    def add(check_id: str) -> None:
        if check_id not in chosen:
            chosen.append(check_id)

    def gate_of(check: Check) -> Check | None:
        return by_id.get(check.covered_by) if check.kind == "unit" and check.covered_by else None

    if sel.targeted:
        for check_id in sel.checks:
            check = by_id[check_id]
            if explicit_tiers and not in_tier(check):
                omitted.append(check_id)
                continue
            gate = gate_of(check) if sel.gate else None
            add(gate.id if gate else check.id)
            if gate:
                notes.append(f"{check.id}: replaced by its gate {gate.id} (--gate)")
        for seam_id in sel.seams:
            covering = [c for c in seam_map.checks if _covers_seam(c, seam_id) and in_layer(c)]
            units = [c for c in covering if c.kind == "unit"]
            pool = units if units else covering
            hit = False
            for check in pool:
                if not in_tier(check):
                    omitted.append(check.id)
                    continue
                gate = gate_of(check) if sel.gate else None
                add(gate.id if gate else check.id)
                hit = True
            if not hit:
                notes.append(f"{seam_id}: no check in tiers {'+'.join(tiers)} covers it")
    else:
        candidates = [c for c in seam_map.checks if in_tier(c) and in_layer(c)]
        candidate_ids = {c.id for c in candidates}
        for check in candidates:
            gate = gate_of(check)
            if check.kind == "gate":
                add(check.id)
            elif sel.gate:
                if gate is not None and in_tier(gate):
                    add(gate.id)
                    notes.append(f"{check.id}: covered by {gate.id} (--gate)")
            elif gate is None:
                add(check.id)
            elif gate.id in candidate_ids:
                notes.append(f"{check.id}: covered by {gate.id} (its records come from the gate)")
            elif in_tier(gate):
                add(gate.id)
                notes.append(f"{check.id}: replaced by its covering gate {gate.id}")
            else:
                add(check.id)

    pulled: set[str] = set()
    queue = list(chosen)
    while queue:
        current = by_id[queue.pop(0)]
        for req in current.requires:
            if req not in chosen:
                chosen.append(req)
                pulled.add(req)
                queue.append(req)
                notes.append(f"{req}: pulled in, {current.id} requires it")

    if not chosen:
        raise SelectionError("the selection matches no check" + (f" ({'; '.join(notes)})" if notes else ""))
    if sel.targeted and not explicit_tiers:  # an explicit --check may sit in any tier: name the tiers it runs
        tiers = tuple(t for t in TIERS if any(by_id[i].tier == t for i in chosen))
    return Plan(_order([by_id[i] for i in chosen], seam_map), tiers, pulled, notes, sorted(set(omitted)))


def _order(checks: list[Check], seam_map: SeamMap) -> list[Check]:
    """Fast, slow, ns; catalog order within a tier; every prerequisite first (a stable topological order)."""
    catalog = {c.id: i for i, c in enumerate(seam_map.checks)}
    rank = {tier: i for i, tier in enumerate(TIERS)}
    pending = sorted(checks, key=lambda c: (rank[c.tier], catalog[c.id]))
    ids = {c.id for c in pending}
    placed: list[Check] = []
    done: set[str] = set()
    while pending:
        for index, check in enumerate(pending):
            if all(r in done or r not in ids for r in check.requires):
                placed.append(check)
                done.add(check.id)
                del pending[index]
                break
        else:  # a cycle: the loader refuses those, so this is defensive
            placed.extend(pending)
            break
    return placed
