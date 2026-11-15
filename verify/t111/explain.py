"""``python3 -I verify/run.py --explain <seam-id | check-id>``: one record of the map in plain text (D37).

For an agent that wants to orient before it runs anything. A seam id (``L3-L4:chat-relay``) prints both sides with
their files, the side to read first, the spec sections, the symptom, every check that proves it (tier, strength, the
exact command that re-runs it through the runner, the native command and its cost), every fake on it and what it
mirrors, and its gap. A check id (``l4.smoke``) prints its command, tier, strength, locks, needs and prerequisites, the
gate that covers it (or the units a gate covers), the seams and phases it proves and the fakes it is the parity check
of. An unknown id exits 2 with the closest ids. Read-only, deterministic.
"""

from __future__ import annotations

import argparse
import difflib
import sys
from pathlib import Path

from .lint import load_map_reporting, proving_checks
from .matrix import seam_strength
from .record import default_rerun
from .schema import Check, Fake, Seam, SeamMap, Side
from .summary import layer_tag

_RERUN_FILLS = ("{id}", "{test}")


def side_text(side: Side) -> str:
    """``L4`` / ``L5/L6/L7`` / ``X``."""
    return "/".join("X" if x == "X" else f"L{x}" for x in side.layer) or "-"


def runner_command(check_id: str, scratch: str) -> str:
    """The command that re-runs one check through the runner."""
    return f"python3 -I verify/run.py --scratch {scratch} --check {check_id}"


def native_command(check: Check) -> str:
    """The check's own re-run command: its template (``{test}`` and ``{id}`` stay as placeholders) or its ``cmd``."""
    if not check.rerun:
        return default_rerun(check)
    text = check.rerun.replace("{check}", check.id).replace("{root}", ".")
    for fill in _RERUN_FILLS:
        text = text.replace(fill, "<" + fill.strip("{}") + ">")
    return text


def phases_for(check: Check, seam_id: str) -> list[str]:
    """The phase prefixes of ``check`` that map to ``seam_id``."""
    return [prefix for prefix, seams in check.phases.items() if seam_id in seams]


def explain_seam(seam_map: SeamMap, seam: Seam, scratch: str) -> str:
    """The plain-text record of one seam."""
    lines = [f"seam {seam.id}  {layer_tag(seam.layers())}  status: {seam.status}  strength: "
             f"{seam_strength(seam_map, seam)}"]
    if seam.symptom:
        lines.append(f"symptom: {seam.symptom}")
    for name in ("a", "b"):
        side = seam.side(name)
        marker = "  <- read first" if seam.read_first == name else ""
        lines.append(f"side {name} ({side_text(side)}){marker}:")
        lines.extend(f"    {f}" for f in side.files)
        if not side.files:
            lines.append("    (no files yet)")
    lines.append("spec: " + ("; ".join(seam.spec) if seam.spec else "-"))
    if seam.status == "planned":
        lines.append("no code yet: a planned seam (its row promotes it when the layer's code lands)")
    proving = proving_checks(seam_map, seam.id)
    lines.append(f"proof ({len(proving)} check{'s' if len(proving) != 1 else ''}):" if proving else "proof: none")
    for check in proving:
        lines.append(f"  {check.id}  {check.kind}  tier {check.tier}  strength {check.strength}")
        lines.append(f"      run:    {runner_command(check.id, scratch)}")
        lines.append(f"      native: {native_command(check)}  (cost: {check.rerun_cost or check.tier})")
        phases = phases_for(check, seam.id)
        if phases:
            lines.append(f"      phases: {', '.join(phases)}")
        if check.covered_by:
            lines.append(f"      covered by gate {check.covered_by} (a full run runs the gate instead)")
    fakes = seam_map.fakes_on(seam.id)
    if fakes:
        lines.append(f"fakes ({len(fakes)}):")
        for fake in fakes:
            lines.append("  " + fake_text(seam_map, seam, fake))
    else:
        lines.append("fakes: none")
    lines.append(f"gap: {seam.gap or '-'}")
    return "\n".join(lines) + "\n"


def fake_text(seam_map: SeamMap, seam: Seam, fake: Fake) -> str:
    """One fake: its path, the side it stands in for and its parity check."""
    mirrored = seam.side(fake.mirrors)
    under = "a" if fake.mirrors == "b" else "b"
    parity = ("checked against nothing it mirrors (parity = none: a gap)" if fake.parity == "none"
              else f"parity check {fake.parity}")
    return (f"{fake.path}  mirrors side {fake.mirrors} ({side_text(mirrored)}); tests run against side {under}; "
            f"{parity}")


def explain_check(seam_map: SeamMap, check: Check, scratch: str) -> str:
    """The plain-text record of one check."""
    locks = list(check.locks()) + (["cargo-slot"] if check.cargo else [])
    lines = [f"check {check.id}  {check.kind}  tier {check.tier}  strength {check.strength}  "
             f"{layer_tag(check.layers)}",
             f"cmd: {default_rerun(check)}",
             f"run: {runner_command(check.id, scratch)}",
             f"native re-run: {native_command(check)}  (cost: {check.rerun_cost or check.tier})",
             f"adapter {check.adapter}  timeout {check.timeout_s} s  locks {', '.join(locks) or '-'}  "
             f"needs {', '.join(check.needs) or '-'}  requires {', '.join(check.requires) or '-'}"
             + ("  offline: skipped" if check.offline_skip else "")]
    if check.env:
        lines.append("env: " + " ".join(f"{k}={v}" for k, v in sorted(check.env.items())))
    if check.covered_by:
        lines.append(f"covered by gate {check.covered_by}: a full or --gate run runs the gate and derives this "
                     f"unit's records from its output")
    units = [c.id for c in seam_map.checks if c.covered_by == check.id]
    if units:
        lines.append(f"covers {len(units)} unit(s): {', '.join(units)}")
    seam_ids = list(check.seams) + [s for seams in check.phases.values() for s in seams if s not in check.seams]
    lines.append(f"proves {len(set(seam_ids))} seam(s):" if seam_ids else "proves no seam")
    shown: set[str] = set()
    for sid in seam_ids:
        if sid in shown:
            continue
        shown.add(sid)
        seam = seam_map.seam(sid)
        phases = phases_for(check, sid)
        via = f"  (phases {', '.join(phases)})" if phases else ""
        lines.append(f"  {sid}  {layer_tag(seam.layers()) if seam else '[unknown seam]'}"
                     f"  {seam.status if seam else '-'}{via}")
    parity = [f for f in seam_map.fakes if f.parity == check.id]
    if parity:
        lines.append("parity check of: " + "; ".join(f"{f.path} ({f.seam})" for f in parity))
    known = [k for k in seam_map.known if k.check == check.id]
    for k in known:
        lines.append(f"known: {k.test} {k.condition} {k.kind} ({k.source}): {k.words()}")
    return "\n".join(lines) + "\n"


def main(args: argparse.Namespace, root: Path, map_dir: Path) -> int:
    """``--explain ID``: print the seam's or check's record. Exit 0; 2 for an unknown id or a map that does not load."""
    seam_map = load_map_reporting(map_dir, root, "explain")
    if seam_map is None:
        return 2
    ident = str(args.explain).strip()
    scratch = str(args.scratch) if getattr(args, "scratch", None) else "DIR"
    seam = seam_map.seam(ident)
    if seam is not None:
        sys.stdout.write(explain_seam(seam_map, seam, scratch))
        return 0
    check = seam_map.check(ident)
    if check is not None:
        sys.stdout.write(explain_check(seam_map, check, scratch))
        return 0
    known = [s.id for s in seam_map.seams] + [c.id for c in seam_map.checks]
    close = difflib.get_close_matches(ident, known, n=5, cutoff=0.5)
    partial = [k for k in known if ident and ident in k and k not in close][:5]
    print(f"--explain: no seam or check with id {ident!r}", file=sys.stderr)
    if close or partial:
        print("closest: " + ", ".join(close + partial), file=sys.stderr)
    return 2
