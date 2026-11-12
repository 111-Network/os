"""The command line of ``python3 -I verify/run.py`` (options.md §3.1) and its dispatch.

Run::

    python3 -I verify/run.py --scratch DIR [--layer 3,4] [--seam ID] [--check ID] [--tier fast,slow,ns] [--gate]
                             [--require] [--keep] [--cargo-slots 1|2] [--jobs N] [--offline] [--ci]
                             [--shared-box] [--lock-wait S] [--log-cap MB]

Commands that run no check: ``--list`` (the plan: tiers, locks, est. time), ``--lint``, ``--matrix``,
``--changed [REF]``, ``--explain ID``. The last four live in ``t111.lint``, ``t111.matrix``, ``t111.changed`` and
``t111.explain`` (row X4); each exposes ``main(args, root, map_dir) -> int`` and, until it exists, the command prints
``not built yet (row X4)`` and exits 2.

Exit codes: 0 green (also green with SKIPPED), 1 red (also every run a signal stopped: SIGINT, SIGTERM, SIGHUP; and a
runner error), 2 usage error (bad flags, an unknown id, a map that does not load, a selection that matches nothing).
"""

from __future__ import annotations

import argparse
import importlib
import os
import sys
from pathlib import Path
from typing import Sequence

from . import engine, locks as locks_mod
from .record import read_results
from .run import LOG_CAP_MB
from .schema import TIERS, SchemaError, load_map
from .select import Plan, Selection, SelectionError, select
from .summary import EXIT_RED, EXIT_USAGE, duration

VERIFY_DIR = Path(__file__).resolve().parent.parent
"""``verify/``: the default map dir."""

REPO_ROOT = VERIFY_DIR.parent
"""The repo root: the default ``--root``."""

X4_COMMANDS: tuple[str, ...] = ("lint", "matrix", "changed", "explain")
"""The commands row X4 implements, each as module ``t111.<name>`` with ``main(args, root, map_dir) -> int``."""


def _csv(value: str) -> list[str]:
    return [part.strip() for part in value.split(",") if part.strip()]


def _positive(value: str) -> float:
    try:
        number = float(value)
    except ValueError:
        raise argparse.ArgumentTypeError(f"not a number: {value!r}") from None
    if not number > 0:
        raise argparse.ArgumentTypeError(f"must be > 0: {value!r}")
    return number


def parser() -> argparse.ArgumentParser:
    """The argument parser (every flag of options.md §3.1, plus ``--jobs``)."""
    p = argparse.ArgumentParser(
        prog="verify/run.py",
        description="Run any slice of the repo's gates and harnesses under the box rules; one JSON record per result "
                    "in <scratch>/<run-id>/results.jsonl, a summary on stdout.",
        epilog="Exit: 0 VERIFY GREEN (also with SKIPPED), 1 VERIFY RED (also a run stopped by SIGINT, SIGTERM or "
               "SIGHUP: its checks are killed and its leftovers swept), 2 usage error.")
    sel = p.add_argument_group("selection (default: every check of tiers fast,slow)")
    sel.add_argument("--layer", action="append", default=[], metavar="N[,N]",
                     help="checks that touch these layers (0..9, X); a full run of that slice")
    sel.add_argument("--seam", action="append", default=[], metavar="ID[,ID]",
                     help="the units that cover these seams (the fastest reproduction)")
    sel.add_argument("--check", action="append", default=[], metavar="ID[,ID]", help="exactly these checks")
    sel.add_argument("--tier", action="append", default=[], metavar="T[,T]",
                     help=f"cost tiers: {', '.join(TIERS)} (default fast,slow; ns is opt-in)")
    sel.add_argument("--gate", action="store_true", help="run gates only; unit records come from the gate's output")
    run = p.add_argument_group("run")
    run.add_argument("--scratch", metavar="DIR", type=Path,
                     help="required for a run: every byte it writes lands under DIR/<run-id>/")
    run.add_argument("--require", action="store_true", help="every SKIPPED counts as FAIL (a lock run)")
    run.add_argument("--keep", action="store_true", help="keep every check's scratch, green ones too")
    run.add_argument("--cargo-slots", type=int, choices=(1, 2), default=1,
                     help="cargo checks at once, shared with other runner instances via flock (default 1, max 2)")
    run.add_argument("--jobs", type=int, choices=(1, 2, 3, 4), default=0,
                     help="checks at once (default: the cargo slots)")
    run.add_argument("--offline", action="store_true",
                     help="checks that need the package index are SKIPPED 'offline' up front")
    run.add_argument("--ci", action="store_true", help="CI mode: no progress lines on stderr")
    run.add_argument("--shared-box", action="store_true",
                     help="use beside other writers (agents editing the tree or starting servers): a tree change or a "
                          "look-alike process the run cannot prove its own is SKIPPED 'not attributed', not FAIL "
                          "(also T111_SHARED_BOX=1)")
    run.add_argument("--lock-wait", type=_positive, default=locks_mod.LOCK_WAIT_S, metavar="S",
                     help="seconds a check waits for a lock (another check or run holds it) before it FAILs naming the holder "
                          f"(default {locks_mod.LOCK_WAIT_S:g})")
    run.add_argument("--log-cap", type=_positive, default=LOG_CAP_MB, metavar="MB",
                     help=f"keep at most MB of a check's output in its out.log; the rest is read and dropped, and the "
                          f"record says 'log cut' (default {LOG_CAP_MB:g})")
    cmd = p.add_argument_group("commands that run no check")
    cmd.add_argument("--list", action="store_true", help="print the plan (checks, tiers, locks, est. time)")
    cmd.add_argument("--lint", action="store_true", help="hold the seam map to the tree (row X4)")
    cmd.add_argument("--matrix", action="store_true", help="print matrix.md's tables from the map (row X4)")
    cmd.add_argument("--changed", nargs="?", const="HEAD", metavar="REF",
                     help="the seams the changed paths touch (row X4)")
    cmd.add_argument("--explain", metavar="ID", help="one seam's or check's whole record from the map (row X4)")
    # for the runner's own tests: another tree, another map, a shorter kill grace, other ns ports
    p.add_argument("--root", type=Path, default=REPO_ROOT, help=argparse.SUPPRESS)
    p.add_argument("--map", dest="map_dir", type=Path, default=VERIFY_DIR, help=argparse.SUPPRESS)
    p.add_argument("--kill-grace", type=float, default=None, help=argparse.SUPPRESS)
    p.add_argument("--ns-ports", default=None, help=argparse.SUPPRESS)
    return p


def selection(args: argparse.Namespace) -> Selection:
    """The ``Selection`` of the parsed flags (comma lists and repeats both work)."""
    def flat(values: list[str]) -> tuple[str, ...]:
        out: list[str] = []
        for value in values:
            for part in _csv(value):
                if part not in out:
                    out.append(part)
        return tuple(out)

    return Selection(layers=flat(args.layer), seams=flat(args.seam), checks=flat(args.check), tiers=flat(args.tier),
                     gate=args.gate)


def x4_command(name: str, args: argparse.Namespace) -> int:
    """Dispatch ``--lint``/``--matrix``/``--changed``/``--explain`` to row X4's module (import guard, no stub)."""
    try:
        module = importlib.import_module(f"{__package__}.{name}")
    except ModuleNotFoundError as exc:
        if exc.name != f"{__package__}.{name}":
            raise
        print(f"--{name}: not built yet (row X4)", file=sys.stderr)
        return EXIT_USAGE
    return int(module.main(args, args.root.resolve(), args.map_dir.resolve()))


def estimates(scratch: Path | None) -> dict[str, float]:
    """Check id -> its duration in the most recent run under ``scratch`` that ran it (for ``--list``)."""
    found: dict[str, float] = {}
    if scratch is None or not scratch.is_dir():
        return found
    for run_dir in sorted((d for d in scratch.iterdir() if d.is_dir()), reverse=True):
        results = run_dir / "results.jsonl"
        if not results.is_file():
            continue
        try:
            rows = read_results(results)
        except (OSError, ValueError):
            continue
        for row in rows:
            check, took = row.get("check"), row.get("duration_s")
            if row.get("kind") == "check" and check and isinstance(took, (int, float)) and check not in found:
                found[check] = float(took)
    return found


def render_list(plan: Plan, args: argparse.Namespace) -> str:
    """The ``--list`` text: one row per check in run order, then the selection notes and the total estimate."""
    last = estimates(args.scratch)
    jobs = args.jobs or args.cargo_slots
    rows = [("#", "check", "kind", "tier", "locks", "needs", "requires", "est")]
    total = 0.0
    unknown = 0
    for n, check in enumerate(plan.checks, 1):
        held = list(check.locks()) + (["cargo-slot"] if check.cargo else [])
        if check.id in last:
            est = f"~{duration(last[check.id])} (last run)"
            total += last[check.id]
        else:
            est = f"? (timeout {duration(check.timeout_s)})"
            unknown += 1
        name = check.id + (" (pulled in)" if check.id in plan.pulled else "")
        rows.append((str(n), name, check.kind, check.tier, ",".join(held) or "-", ",".join(check.needs) or "-",
                     ",".join(check.requires) or "-", est))
    widths = [max(len(r[i]) for r in rows) for i in range(len(rows[0]))]
    lines = [f"plan: {len(plan.checks)} checks, tiers {'+'.join(plan.tiers)}, cargo slots {args.cargo_slots}, "
             f"jobs {jobs}, lock dir {locks_mod.lock_dir()}"]
    for row in rows:
        lines.append("  " + "  ".join(cell.ljust(widths[i]) for i, cell in enumerate(row)).rstrip())
    for note in plan.notes:
        lines.append(f"note: {note}")
    if plan.omitted:
        lines.append(f"note: also covering the selection, in tiers not asked for: {', '.join(plan.omitted)}")
    tail = f"est. total: ~{duration(total)} serial over {len(plan.checks) - unknown} check(s) with a last run"
    if unknown:
        tail += f"; {unknown} never run here"
    lines.append(tail)
    lines.append("nothing was run")
    return "\n".join(lines) + "\n"


def main(argv: Sequence[str] | None = None) -> int:
    """Parse, load the map, select, then list, dispatch or run. Returns the exit code."""
    raw = list(sys.argv[1:] if argv is None else argv)
    p = parser()
    args = p.parse_args(raw)
    commands = [name for name in ("list", *X4_COMMANDS) if getattr(args, name) not in (None, False)]
    if len(commands) > 1:
        p.error(f"choose one of --{', --'.join(commands)}")
    if commands and commands[0] in X4_COMMANDS:
        return x4_command(commands[0], args)
    try:
        seam_map = load_map(args.map_dir, display_root=args.root)
    except SchemaError as exc:
        for error in exc.errors:
            print(f"map: {error}", file=sys.stderr)
        print(f"map: {len(exc.errors)} schema error(s); nothing was run", file=sys.stderr)
        return EXIT_USAGE
    try:
        plan = select(seam_map, selection(args))
    except SelectionError as exc:
        print(f"verify/run.py: {exc}", file=sys.stderr)
        return EXIT_USAGE
    if args.list:
        sys.stdout.write(render_list(plan, args))
        return 0
    if args.scratch is None:
        p.error("--scratch DIR is required for a run (every byte the run writes lands under it)")
    root = args.root.resolve()
    scratch = args.scratch.resolve()
    try:
        scratch.relative_to(root)
        inside = True
    except ValueError:
        inside = False
    if inside:
        p.error(f"--scratch must be outside the tree ({root})")
    ports = None
    if args.ns_ports is not None:
        try:
            ports = tuple(int(x) for x in _csv(args.ns_ports))
        except ValueError:
            p.error("--ns-ports takes a comma list of port numbers")
    shared = args.shared_box or os.environ.get("T111_SHARED_BOX", "") == "1"
    cfg = engine.Config(root=root, scratch=scratch, seam_map=seam_map, plan=plan, cargo_slots=args.cargo_slots,
                        jobs=args.jobs, keep=args.keep, offline=args.offline, require=args.require, ci=args.ci,
                        shared_box=shared, lock_wait_s=args.lock_wait, log_cap_mb=args.log_cap,
                        argv=["verify/run.py", *raw])
    if args.kill_grace is not None:
        cfg.kill_grace_s = args.kill_grace
    if ports is not None:
        cfg.ns_ports = ports
    try:
        return engine.execute(cfg).exit_code
    except KeyboardInterrupt:  # Ctrl-C before the engine took over the stop signals: nothing ran
        engine.interrupted_before_start(None, "SIGINT")
        return EXIT_RED
