"""``python3 -I verify/run.py --changed [REV]``: which seams and checks the changed paths touch (options.md §5).

``git diff --name-only REV`` (default ``HEAD``: the working tree against the last commit; a range ``A..B`` works too)
plus the untracked files gives the changed paths. Each path is mapped through the seams' side files (an exact file, a
directory prefix or a glob) and through the checks' own files (a test file or script a check's ``cmd`` claims). The
output names the seams touched, the checks that claim a changed file, the paths nothing maps, and one ``run.py``
command that runs the units covering them (the plan the runner would build, so ``--list`` agrees). A change to the map
itself or to ``verify/t111`` points at ``--lint`` and the unit tests. Read-only: git is run with an explicit argv, no
lock, no index refresh.
"""

from __future__ import annotations

import argparse
import os
import shlex
import subprocess
import sys
from pathlib import Path

from .explain import side_text
from .lint import claimed_files, load_map_reporting, matches_entry
from .schema import Check, Seam, SeamMap
from .select import Selection, SelectionError, select

GIT_TIMEOUT_S = 60
MAX_LISTED = 25
"""At most this many unmapped paths are listed; the rest are counted."""


class GitError(Exception):
    """git failed or the revision is not acceptable."""


def _git(root: Path, argv: list[str]) -> bytes:
    env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    env["GIT_OPTIONAL_LOCKS"] = "0"
    try:
        proc = subprocess.run(["git", "-c", "core.quotepath=off", *argv], cwd=root, env=env, capture_output=True,
                              timeout=GIT_TIMEOUT_S, start_new_session=True, check=False)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise GitError(f"git {argv[0]}: {exc}") from exc
    if proc.returncode != 0:
        raise GitError(f"git {' '.join(argv[:3])} failed: {proc.stderr.decode('utf-8', 'replace').strip()}")
    return proc.stdout


def _split(raw: bytes) -> list[str]:
    return [p.decode("utf-8", "replace") for p in raw.split(b"\0") if p]


def changed_paths(root: Path, rev: str) -> tuple[list[str], list[str]]:
    """``(tracked, untracked)``: the paths that differ from ``rev`` (sorted), and untracked files not ignored
    (none for a ``A..B`` range). Raises ``GitError``."""
    if rev.startswith("-") or "\0" in rev or not rev.strip():
        raise GitError(f"not a revision: {rev!r}")
    tracked = sorted(set(_split(_git(root, ["diff", "--name-only", "-z", rev, "--"]))))
    untracked: list[str] = []
    if ".." not in rev:
        untracked = sorted(set(_split(_git(root, ["ls-files", "-z", "--others", "--exclude-standard"]))) - set(tracked))
    return tracked, untracked


def seams_for_path(seam_map: SeamMap, path: str) -> list[tuple[Seam, str]]:
    """The ``(seam, side)`` pairs whose side files contain ``path``."""
    out: list[tuple[Seam, str]] = []
    for seam in seam_map.seams:
        for name in ("a", "b"):
            if any(matches_entry(entry, path) for entry in seam.side(name).files):
                out.append((seam, name))
                break
    return out


def checks_for_paths(seam_map: SeamMap, root: Path, paths: list[str]) -> dict[str, list[str]]:
    """Check id -> the changed paths its ``cmd`` claims (a script, a test file, a directory it runs)."""
    out: dict[str, list[str]] = {}
    for check in seam_map.checks:
        hit = sorted(claimed_files(check, root, paths) & set(paths))
        if hit:
            out[check.id] = hit
    return out


def seams_of_check(check: Check) -> list[str]:
    """The seam ids a check proves (``seams`` then the ``phases`` seams), without repeats."""
    out = list(check.seams)
    for seams in check.phases.values():
        out.extend(s for s in seams if s not in out)
    return out


def analyse(seam_map: SeamMap, root: Path, paths: list[str], map_files: list[str]) -> dict:
    """Map ``paths`` to seams and checks. Keys: ``seam_paths`` (seam id -> [(path, side)]), ``check_paths``,
    ``unmapped``, ``map_touched`` (paths of the map or the runner), ``seams`` and ``checks`` (the ids to run)."""
    seam_paths: dict[str, list[tuple[str, str]]] = {}
    for path in paths:
        for seam, side in seams_for_path(seam_map, path):
            seam_paths.setdefault(seam.id, []).append((path, side))
    check_paths = checks_for_paths(seam_map, root, paths)
    map_touched = [p for p in paths if p in map_files or p.startswith("verify/t111/") or p.startswith("verify/tests/")
                   or p == "verify/run.py"]
    mapped = {p for hits in seam_paths.values() for p, _ in hits} | {p for hits in check_paths.values() for p in hits}
    unmapped = [p for p in paths if p not in mapped and p not in map_touched]
    seam_ids = [s.id for s in seam_map.seams if s.id in seam_paths]
    for cid in check_paths:
        check = seam_map.check(cid)
        for sid in seams_of_check(check) if check else []:
            if sid not in seam_ids and seam_map.seam(sid) is not None:
                seam_ids.append(sid)
    return {"seam_paths": seam_paths, "check_paths": check_paths, "unmapped": unmapped, "map_touched": map_touched,
            "seams": seam_ids, "checks": list(check_paths)}


def render(seam_map: SeamMap, found: dict, rev: str, n_tracked: int, n_untracked: int, scratch: str) -> str:
    """The report text for one analysis."""
    total = n_tracked + n_untracked
    lines = [f"changed against {rev}: {total} path(s) ({n_tracked} tracked, {n_untracked} untracked)"]
    if not total:
        lines.append("nothing changed: nothing to run")
        return "\n".join(lines) + "\n"
    if found["seam_paths"]:
        lines.append(f"seams touched by a side file ({len(found['seam_paths'])}):")
        for sid, hits in found["seam_paths"].items():
            seam = seam_map.seam(sid)
            assert seam is not None
            shown = ", ".join(f"{p} (side {side}, {side_text(seam.side(side))})" for p, side in hits[:3])
            more = f" and {len(hits) - 3} more" if len(hits) > 3 else ""
            lines.append(f"  {sid} [{seam.status}] <- {shown}{more}")
    if found["check_paths"]:
        lines.append(f"checks that claim a changed file ({len(found['check_paths'])}):")
        for cid, hits in found["check_paths"].items():
            lines.append(f"  {cid} <- {', '.join(hits[:3])}{f' and {len(hits) - 3} more' if len(hits) > 3 else ''}")
    if found["map_touched"]:
        lines.append(f"the map or the runner changed ({len(found['map_touched'])} path(s), e.g. {found['map_touched'][0]}):")
        lines.append("  python3 -I verify/run.py --lint")
        lines.append("  python3 -I -m unittest discover -s verify/tests -t .")
    if found["unmapped"]:
        shown = found["unmapped"][:MAX_LISTED]
        lines.append(f"no seam and no check maps {len(found['unmapped'])} path(s): " + ", ".join(shown)
                     + (f" ... and {len(found['unmapped']) - MAX_LISTED} more" if len(found["unmapped"]) > MAX_LISTED
                        else ""))
    seams, checks = found["seams"], found["checks"]
    if not seams and not checks:
        lines.append("no seam or check to run for these paths")
        return "\n".join(lines) + "\n"
    try:
        plan = select(seam_map, Selection(seams=tuple(seams), checks=tuple(checks)))
    except SelectionError as exc:
        lines.append(f"no check covers them in the default tiers: {exc}")
        return "\n".join(lines) + "\n"
    argv = ["python3", "-I", "verify/run.py", "--scratch", scratch]
    if seams:
        argv += ["--seam", ",".join(seams)]
    if checks:
        argv += ["--check", ",".join(checks)]
    lines.append(f"run ({len(plan.checks)} check(s), tiers {'+'.join(plan.tiers)}):")
    lines.append("  " + " ".join(shlex.quote(a) for a in argv))
    lines.append("  checks: " + ", ".join(f"{c.id} ({c.tier})" for c in plan.checks))
    uncovered = [sid for sid in seams if not any(sid in seams_of_check(c) for c in plan.checks)]
    if uncovered:
        lines.append("  no check in these tiers proves: " + ", ".join(uncovered))
    if plan.omitted:
        lines.append(f"  also covering, in a tier not asked for (add --tier fast,slow,ns): {', '.join(plan.omitted)}")
    return "\n".join(lines) + "\n"


def main(args: argparse.Namespace, root: Path, map_dir: Path) -> int:
    """``--changed [REV]``: print the report. Exit 0, or 2 when git fails or the map does not load."""
    seam_map = load_map_reporting(map_dir, root, "changed")
    if seam_map is None:
        return 2
    rev = args.changed or "HEAD"
    try:
        tracked, untracked = changed_paths(root, rev)
    except GitError as exc:
        print(f"--changed: {exc}", file=sys.stderr)
        return 2
    paths = sorted(set(tracked) | set(untracked))
    map_files = []
    for name in ("seams.toml", "checks.toml"):
        try:
            map_files.append((map_dir / name).resolve().relative_to(root.resolve()).as_posix())
        except ValueError:
            pass
    found = analyse(seam_map, root, paths, map_files)
    scratch = str(args.scratch) if getattr(args, "scratch", None) else "DIR"
    sys.stdout.write(render(seam_map, found, rev, len(tracked), len(untracked), scratch))
    return 0
