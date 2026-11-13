"""``python3 -I verify/run.py --matrix``: the seam x evidence matrix, generated from the map (options.md §3.1).

``plan/cross-layer/test-system/matrix.md`` was written by hand. This prints its tables from ``verify/seams.toml`` and
``verify/checks.toml`` instead, so they cannot drift: per seam its status, its strength (the best strength of the checks
that prove it: R running system, I integration, F fake, C static, N none), its checks with their tier, and its gap; the
totals per strength and status; the built seams without running-system evidence; the fakes checked against nothing.

With a ``results.jsonl`` (``--results FILE``, or without it the newest ``<scratch>/<run-id>/results.jsonl`` under
``--scratch``) every check also shows its last status for that seam: ``PASS``, ``FAIL``, ``SKIPPED`` (the worst of its
records) or ``not run``. When that run did not run every check of the map (a ``--seam`` / ``--check`` / ``--layer``
selection, or a default run that leaves the ``ns`` tier out) the header says ``last run was a selection of N checks``, so
``not run`` is not read as never run. A results file that is missing, unreadable, not JSON lines or whose run record has a
``checks``, ``tiers`` or ``via`` of the wrong shape exits 2 with a named error. Output is Markdown, deterministic (no clock), and writes nothing.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path
from typing import Any, Iterable, Mapping

from .lint import load_map_reporting, proving_checks
from .record import read_results
from .schema import STRENGTHS, Check, Seam, SeamMap

STRENGTH_NAMES: Mapping[str, str] = {
    "R": "R running system",
    "I": "I integration with the real peer",
    "F": "F fake of the other side",
    "C": "C compiled or static only",
    "N": "N none",
}
"""The legend of the strength column (matrix.md)."""

STATUS_RANK: Mapping[str, int] = {"FAIL": 0, "SKIPPED": 1, "PASS": 2}
"""Worst first: a check's status is the worst of its records."""

NOT_RUN = "not run"


def best_strength(checks: Iterable[Check]) -> str:
    """The best strength among ``checks`` (R > I > F > C > N); ``N`` when there are none."""
    have = {c.strength for c in checks}
    for strength in STRENGTHS:
        if strength in have:
            return strength
    return "N"


def seam_strength(seam_map: SeamMap, seam: Seam) -> str:
    """The strength of one seam: its best proving check (a planned seam is ``N`` whatever is declared)."""
    if seam.status == "planned":
        return "N"
    return best_strength(proving_checks(seam_map, seam.id))


def worst(statuses: Iterable[str]) -> str | None:
    """The worst of a set of statuses, or None when empty."""
    got = [s for s in statuses if s in STATUS_RANK]
    return min(got, key=lambda s: STATUS_RANK[s]) if got else None


def _check_run_record(row: Mapping[str, Any]) -> None:
    """A run record's plan fields have the shapes ``engine.run_record`` writes (``checks`` and ``tiers`` lists, ``via`` an
    object, each possibly absent or null); anything else is a ``ValueError`` (``main`` reports it, exit 2) rather than a
    note read from the wrong thing."""
    for key, kind, name in (("checks", list, "a list"), ("tiers", list, "a list"), ("via", Mapping, "an object")):
        value = row.get(key)
        if value is not None and not isinstance(value, kind):
            raise ValueError(f"the run record's `{key}` is not {name}: {str(value)[:60]!r}")


class Results:
    """The check records of one ``results.jsonl``, indexed for the matrix."""

    def __init__(self, rows: Iterable[Mapping[str, Any]]):
        self.run: Mapping[str, Any] = {}
        self.records: list[Mapping[str, Any]] = []
        for row in rows:
            if not isinstance(row, Mapping):
                raise ValueError(f"a line of the results is not a JSON object: {str(row)[:60]!r}")
            if row.get("kind") == "run":
                _check_run_record(row)
            if row.get("kind") == "run" and not self.run:
                self.run = row
            elif row.get("kind") == "check":
                self.records.append(row)

    def _owner(self, row: Mapping[str, Any]) -> str:
        check = row.get("check")
        if isinstance(check, str) and check:
            return check
        ident = str(row.get("id", ""))
        if ident.startswith("hygiene/"):
            return ident.split("/")[1]
        return ident.split("/")[0]

    def selection_note(self, seam_map: SeamMap) -> str:
        """``last run was a selection of N checks (...)`` when the run record's plan leaves out checks of the map (a
        gate stands in for the units it covers); an empty string when it ran them all or the record has no plan."""
        planned = self.run.get("checks")
        if not isinstance(planned, list):
            return ""
        ran = {str(c) for c in planned} | {str(c) for c in (self.run.get("via") or {})}
        gates = {c for c in ran if (chk := seam_map.check(c)) is not None and chk.kind == "gate"}
        missing = [c.id for c in seam_map.checks if c.id not in ran and c.covered_by not in gates]
        if not missing:
            return ""
        tiers = self.run.get("tiers")
        where = f", tiers {'+'.join(str(t) for t in tiers)}" if isinstance(tiers, list) and tiers else ""
        return (f"Last run was a selection of {len(planned)} checks (of {len(seam_map.checks)} in the map{where}); "
                f"{len(missing)} not in it show `not run`.")

    def status(self, check_id: str, seam_id: str | None = None) -> str:
        """The last status of ``check_id`` (for ``seam_id`` when records carry it), or ``NOT_RUN``."""
        rows = [r for r in self.records if self._owner(r) == check_id]
        if seam_id is not None:
            narrowed = [r for r in rows if seam_id in (r.get("seams") or [])]
            rows = narrowed or rows
        return worst(str(r.get("status")) for r in rows) or NOT_RUN


def latest_results(scratch: Path | None) -> Path | None:
    """The newest ``<scratch>/<run-id>/results.jsonl`` (run ids sort by time), or None."""
    if scratch is None or not scratch.is_dir():
        return None
    for run_dir in sorted((d for d in scratch.iterdir() if d.is_dir()), reverse=True):
        path = run_dir / "results.jsonl"
        if path.is_file():
            return path
    return None


def _section(seam: Seam) -> tuple[int, str]:
    """The block a seam is listed under: planned last, cross-cutting before them, else by the id's first layer."""
    if seam.status == "planned":
        return 99, "Planned (no code yet)"
    m = re.match(r"^L(\d)", seam.id)
    if not m:
        return 90, "Cross-cutting"
    return int(m.group(1)), f"Layer {m.group(1)}"


def layers_text(seam: Seam) -> str:
    """``3↔4``: the layers a seam joins, ascending (``X`` last)."""
    layers = seam.layers()
    ints = sorted(x for x in layers if isinstance(x, int))
    return "↔".join([str(x) for x in ints] + ["X"] * (any(x == "X" for x in layers)))


def _cell(text: str) -> str:
    return text.replace("|", "\\|").replace("\n", " ").strip()


def _checks_cell(seam_map: SeamMap, seam: Seam, results: Results | None) -> str:
    parts = []
    for check in proving_checks(seam_map, seam.id):
        text = f"`{check.id}` [{check.tier}, {check.strength}]"
        if results is not None:
            text += f" {results.status(check.id, seam.id)}"
        parts.append(text)
    return "; ".join(parts) if parts else "-"


def _table(rows: list[list[str]]) -> list[str]:
    head, body = rows[0], rows[1:]
    out = ["| " + " | ".join(head) + " |", "|" + "|".join("---" for _ in head) + "|"]
    out.extend("| " + " | ".join(_cell(c) for c in row) + " |" for row in body)
    return out


def render(seam_map: SeamMap, results: Results | None = None, *, results_path: str = "") -> str:
    """The matrix as Markdown text (ends with a newline)."""
    lines = ["# Seam x evidence matrix (generated from verify/seams.toml and verify/checks.toml)", ""]
    if results is not None:
        run = results.run
        head = run.get("head")
        extra = f" (run {run['run_id']}, head {str(head)[:9]}{', dirty tree' if run.get('dirty') else ''})" if run else ""
        lines += [f"Last status from `{results_path}`{extra}; a check's status is the worst of its records.", ""]
        note = results.selection_note(seam_map)
        if note:
            lines += [note, ""]
    lines += ["Strength = the best check that proves the seam: " + ", ".join(STRENGTH_NAMES[s] for s in STRENGTHS)
              + ". `[tier, strength]` follow each check.", ""]
    by_section: dict[tuple[int, str], list[Seam]] = {}
    for seam in seam_map.seams:
        by_section.setdefault(_section(seam), []).append(seam)
    header = ["seam", "layers", "status", "str", "checks" + (" (last status)" if results else ""), "gap"]
    for key in sorted(by_section):
        lines += [f"## {key[1]}", ""]
        rows = [header]
        for seam in by_section[key]:
            layers = layers_text(seam)
            rows.append([f"`{seam.id}`", layers, seam.status, seam_strength(seam_map, seam),
                         _checks_cell(seam_map, seam, results), seam.gap or "-"])
        lines += _table(rows) + [""]
    lines += totals(seam_map, results)
    return "\n".join(lines).rstrip("\n") + "\n"


def totals(seam_map: SeamMap, results: Results | None) -> list[str]:
    """The totals and gaps blocks."""
    lines = ["## Totals", ""]
    counts = {s: 0 for s in STRENGTHS}
    for seam in seam_map.seams:
        if seam.status != "planned":
            counts[seam_strength(seam_map, seam)] += 1
    lines += _table([["strength", "built and stub seams"]] + [[STRENGTH_NAMES[s], str(counts[s])] for s in STRENGTHS])
    lines += [""]
    status_counts = {s: sum(1 for x in seam_map.seams if x.status == s) for s in ("built", "stub", "planned")}
    lines += _table([["status", "seams"]] + [[s, str(n)] for s, n in status_counts.items()]
                    + [["all", str(len(seam_map.seams))]])
    lines += [""]
    if results is not None:
        by_status: dict[str, int] = {}
        for check in seam_map.checks:
            status = results.status(check.id)
            by_status[status] = by_status.get(status, 0) + 1
        order = ["PASS", "FAIL", "SKIPPED", NOT_RUN]
        lines += _table([["last status", "checks"]] + [[s, str(by_status[s])] for s in order if s in by_status])
        lines += [""]
    lines += ["## Gaps", ""]
    weak = [s for s in seam_map.seams if s.status == "built" and seam_strength(seam_map, s) != "R"]
    built = sum(1 for s in seam_map.seams if s.status == "built")
    lines.append(f"Built seams without running-system evidence ({len(weak)} of {built}): "
                 + (", ".join(f"`{s.id}`" for s in weak) or "none") + ".")
    lines.append("")
    none = [f for f in seam_map.fakes if f.parity == "none"]
    lines.append(f"Fakes checked against nothing they mirror ({len(none)} of {len(seam_map.fakes)}): "
                 + (", ".join(f"`{f.path}` ({f.seam})" for f in none) or "none") + ".")
    lines.append("")
    return lines


def main(args: argparse.Namespace, root: Path, map_dir: Path) -> int:
    """``--matrix``: print the matrix; the last statuses come from ``--results FILE`` or the newest run under
    ``--scratch`` when one exists. Exit 0, or 2 when the map or the results file does not load."""
    seam_map = load_map_reporting(map_dir, root, "matrix")
    if seam_map is None:
        return 2
    path: Path | None = getattr(args, "results", None)
    if path is None:
        path = latest_results(getattr(args, "scratch", None))
    results: Results | None = None
    if path is not None:
        try:
            results = Results(read_results(Path(path)))
        except (OSError, ValueError) as exc:  # a missing file, a directory, bad UTF-8, bad JSON, a non-object line, a malformed run record
            print(f"--matrix: cannot read results {path}: {exc}", file=sys.stderr)
            return 2
    sys.stdout.write(render(seam_map, results, results_path=str(path) if path else ""))
    return 0
