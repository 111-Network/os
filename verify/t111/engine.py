"""One run: the scheduler that runs a plan's checks under the box rules and writes the records (options.md §3.2–3.3).

``execute(cfg)`` creates ``<scratch>/<run-id>/`` (run-id = UTC time plus the short HEAD), writes the ``run`` record,
then runs the plan with ``cfg.jobs`` workers (default: the cargo slots, so 1). For each check, in this order:

1. SKIPPED up front when a prerequisite did not pass (``requires <id> (<status>)``), under ``--offline`` for an
   ``offline_skip`` check (``offline``), or for a missing need (``needs <x> (missing)``);
2. its named locks, then a cargo slot when ``cargo = true`` (``t111.locks``);
3. for an ``ns`` check, the port pre-check (``port 11180 held by pid N (<cmdline>)`` = SKIPPED);
4. the process and tree snapshots, the subprocess session (``t111.run``), the leftover scan (``hygiene/<check>``,
   killed) and the tree diff (``hygiene/<check>/tree``);
5. the adapter, the engine's two rules (a timeout is a FAIL; a non-zero exit with nothing parsed is a FAIL), the map
   enrichment, the cascade link ``after``, the ``[[known]]`` rule and ``--require``;
6. the check's scratch is deleted when it has no FAIL (unless ``--keep``); a red one is kept and named in its records.

Records stream to ``results.jsonl`` as checks finish; ``summary.txt`` and stdout get the summary in plan order.
"""

from __future__ import annotations

import datetime as _dt
import os
import shutil
import signal
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, TextIO

from . import adapters, hygiene, locks as locks_mod, record, summary
from .adapters import AdapterContext
from .record import Record
from .run import KILL_GRACE_S, Paths, run_check
from .schema import Check, SeamMap
from .select import Plan

DEFAULT_TOOLS: tuple[str, ...] = ("cargo", "pnpm", "node", "python3", "git", "shellcheck", "unshare", "nsenter", "ip",
                                  "curl", "jq", "ss")
"""Tools the run record always reports (plus every tool a selected check needs)."""

TREE_NAMES_MAX = 20
"""How many changed paths a tree-hygiene FAIL names before ``+N more``."""


@dataclass
class Config:
    """Everything one run needs. ``scratch`` is ``--scratch``; every byte the run writes lands under it (the lock
    files of ``t111.locks`` excepted, by design)."""

    root: Path
    scratch: Path
    seam_map: SeamMap
    plan: Plan
    cargo_slots: int = 1
    jobs: int = 0
    keep: bool = False
    offline: bool = False
    require: bool = False
    ci: bool = False
    kill_grace_s: float = KILL_GRACE_S
    ns_ports: tuple[int, ...] = hygiene.NS_PORTS
    lock_dir: Path | None = None
    argv: list[str] = field(default_factory=list)
    out: TextIO | None = None
    err: TextIO | None = None


@dataclass
class Result:
    """What ``execute`` returns: the run dir, every record in plan order, the summary text and the exit code."""

    run_dir: Path
    records: list[Record]
    text: str
    exit_code: int


def utc_stamp() -> str:
    """``20261111T120102Z`` (for the run id only)."""
    return _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def git_head(root: Path) -> str | None:
    """The short HEAD of ``root``, or None."""
    if shutil.which("git") is None:
        return None
    try:
        done = hygiene.helper_run(["git", "--no-optional-locks", "-C", str(root), "rev-parse", "--short", "HEAD"],
                                  cwd=root, run_id="head", timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    head = done.stdout.decode().strip()
    return head if done.returncode == 0 and head else None


def make_run_dir(scratch: Path, head: str | None) -> tuple[str, Path]:
    """Create ``<scratch>/<utc>-<head>`` (``-2``, ``-3`` … on a collision) and return (run id, path)."""
    scratch.mkdir(parents=True, exist_ok=True)
    base = f"{utc_stamp()}-{head or 'nohead'}"
    for n in range(1, 1000):
        run_id = base if n == 1 else f"{base}-{n}"
        path = scratch / run_id
        try:
            path.mkdir()
            return run_id, path
        except FileExistsError:
            continue
    raise RuntimeError(f"cannot create a run dir under {scratch}")


class Engine:
    """Runs one plan. Shared state (statuses, live sessions, records) is guarded by ``_cv``/``_live_lock``."""

    def __init__(self, cfg: Config):
        self.cfg = cfg
        self.jobs = cfg.jobs or cfg.cargo_slots
        self.out = cfg.out or sys.stdout
        self.err = cfg.err or sys.stderr
        self.runner_pid = os.getpid()
        self.subreaper = hygiene.set_subreaper()
        head = git_head(cfg.root)
        self.run_id, self.run_dir = make_run_dir(cfg.scratch, head)
        self.head = head
        self.locks = locks_mod.Locks(cfg.lock_dir or locks_mod.lock_dir(), cfg.cargo_slots)
        self.probes = hygiene.Probes(offline=cfg.offline, run_id=self.run_id, cwd=cfg.root)
        self.facts = {"euid0": os.geteuid() == 0, "offline": cfg.offline}
        self.writer = record.ResultsWriter(self.run_dir / "results.jsonl")
        self._cv = threading.Condition()
        self._live_lock = threading.Lock()
        self._live: dict[str, int] = {}
        self._status: dict[str, str] = {}
        self._records: dict[str, list[Record]] = {}
        self._pending: list[Check] = list(cfg.plan.checks)
        self._started = 0
        self._failed: BaseException | None = None

    # -- the run record ------------------------------------------------------------------------------------------

    def run_record(self) -> dict[str, Any]:
        """The first line of ``results.jsonl``: what the box had, so every skip reason can be checked."""
        plan = self.cfg.plan.checks
        needs = sorted({n for c in plan for n in c.needs if n not in hygiene.Probes.NAMED})
        tools = {t: shutil.which(t) is not None for t in sorted(set(DEFAULT_TOOLS) | set(needs))}
        want_ns = any(c.tier == "ns" or {"userns", "netns"} & set(c.needs) for c in plan)
        tree = hygiene.tree_state(self.cfg.root, self.run_id)
        return {
            "v": record.RECORD_VERSION, "kind": "run", "run_id": self.run_id, "head": self.head,
            "dirty": None if tree is None else bool(tree), "euid": os.geteuid(), "tools": tools,
            "namespaces": self.probes.has("userns") if want_ns else None, "subreaper": self.subreaper,
            "argv": self.cfg.argv, "tiers": list(self.cfg.plan.tiers), "checks": [c.id for c in plan],
            "pulled": sorted(self.cfg.plan.pulled), "cargo_slots": self.cfg.cargo_slots, "jobs": self.jobs,
            "offline": self.cfg.offline, "require": self.cfg.require, "ci": self.cfg.ci, "keep": self.cfg.keep,
            "root": str(self.cfg.root), "run_dir": str(self.run_dir), "started": _dt.datetime.now(
                _dt.timezone.utc).isoformat(timespec="seconds"), "python": sys.version.split()[0],
        }

    # -- scheduling ----------------------------------------------------------------------------------------------

    def _next(self) -> Check | None:
        with self._cv:
            while True:
                if self._failed is not None or not self._pending:
                    return None
                plan_ids = {c.id for c in self.cfg.plan.checks}
                for index, check in enumerate(self._pending):
                    if all(r in self._status or r not in plan_ids for r in check.requires):
                        del self._pending[index]
                        self._started += 1
                        return check
                self._cv.wait()

    def _worker(self) -> None:
        try:
            while True:
                check = self._next()
                if check is None:
                    return
                status, recs = self.process(check)
                with self._cv:
                    self._status[check.id] = status
                    self._records[check.id] = recs
                    self._cv.notify_all()
        except BaseException as exc:  # a runner bug must not hang the other workers
            with self._cv:
                self._failed = exc
                self._cv.notify_all()

    def execute(self) -> Result:
        """Run the plan; return the records, the summary and the exit code."""
        began = time.monotonic()
        self.writer.write(self.run_record())
        workers = [threading.Thread(target=self._worker, name=f"t111-worker-{i}", daemon=True)
                   for i in range(max(1, self.jobs))]
        for worker in workers:
            worker.start()
        try:
            for worker in workers:
                while worker.is_alive():
                    worker.join(0.2)
        except KeyboardInterrupt:
            self.kill_live()
            raise
        finally:
            hygiene.reap_orphans(self.runner_pid)
        if self._failed is not None:
            self.kill_live()
            self.writer.close()
            raise self._failed
        records = [r for c in self.cfg.plan.checks for r in self._records.get(c.id, [])]
        results = self.run_dir / "results.jsonl"
        parity_none = sum(1 for f in self.cfg.seam_map.fakes if f.parity == "none")
        text, code = summary.render(records, n_checks=len(self.cfg.plan.checks), tiers=self.cfg.plan.tiers,
                                    duration_s=time.monotonic() - began, results=str(results),
                                    parity_none=parity_none)
        (self.run_dir / "summary.txt").write_text(text, encoding="utf-8")
        self.writer.close()
        self.out.write(text)
        self.out.flush()
        return Result(self.run_dir, records, text, code)

    def kill_live(self) -> None:
        """SIGKILL the session of every check still running (on Ctrl-C or a runner bug)."""
        with self._live_lock:
            for pid in self._live.values():
                try:
                    os.killpg(pid, signal.SIGKILL)
                except (ProcessLookupError, PermissionError):
                    pass

    def progress(self, check: Check, status: str, recs: list[Record], seconds: float | None) -> None:
        if self.cfg.ci:
            return
        counts = {s: sum(1 for r in recs if r.status == s) for s in record.STATUSES}
        detail = ", ".join(f"{n} {s}" for s, n in counts.items() if n)
        took = f" in {summary.duration(seconds)}" if seconds is not None else ""
        self.err.write(f"t111: {check.id} {status} ({detail}){took}\n")
        self.err.flush()

    # -- one check -----------------------------------------------------------------------------------------------

    def skip_reason(self, check: Check) -> str | None:
        """Why ``check`` is SKIPPED before it starts, or None."""
        with self._cv:
            for req in check.requires:
                status = self._status.get(req)
                if status is not None and status != "PASS":
                    return f"requires {req} ({status})"
        if self.cfg.offline and check.offline_skip:
            return "offline"
        missing = self.probes.missing(check.needs)
        if missing:
            return f"needs {', '.join(missing)} (missing)"
        return None

    def finish(self, check: Check, recs: list[Record], *, paths: Paths | None,
               seconds: float | None) -> tuple[str, list[Record]]:
        """Enrich, apply the known and require rules, write; return the check's status and its records."""
        for rec in recs:
            if rec.seams is None and rec.id.startswith("hygiene/"):
                rec.seams = []
            record.enrich(rec, check, self.cfg.seam_map, root=self.cfg.root)
            rec.duration_s = None if seconds is None else round(seconds, 3)
        record.mark_after(recs)
        for rec in recs:
            record.apply_known(rec, self.cfg.seam_map.known, self.facts)
            if self.cfg.require:
                record.apply_require(rec)
        red = any(r.status == "FAIL" for r in recs)
        kept = paths is not None and paths.scratch.exists() and (red or self.cfg.keep)
        if paths is not None and paths.scratch.exists() and not kept:
            shutil.rmtree(paths.scratch, ignore_errors=True)
        for rec in recs:
            if kept and paths is not None:
                rec.log = rec.log or str(paths.log)
            else:
                rec.log = None
            self.writer.write(rec.to_json())
        if red:
            status = "FAIL"
        elif recs and all(r.status == "SKIPPED" for r in recs):
            status = "SKIPPED"
        else:
            status = "PASS"
        self.progress(check, status, recs, seconds)
        return status, recs

    def process(self, check: Check) -> tuple[str, list[Record]]:
        """Run one check through steps 1–6 of the module docstring."""
        reason = self.skip_reason(check)
        if reason is not None:
            return self.finish(check, [Record(id=check.id, status="SKIPPED", reason=reason)], paths=None, seconds=None)
        adapter = adapters.get(check.adapter)
        if adapter is None:
            return self.finish(check, [Record(id=check.id, status="FAIL", reason=f"unknown adapter {check.adapter!r} "
                                              f"(registered: {', '.join(sorted(adapters.REGISTRY))})")],
                               paths=None, seconds=None)
        with self.locks.for_check(check.locks(), check.cargo):
            if check.tier == "ns":
                held = hygiene.port_holders(self.cfg.ns_ports)
                if held:
                    port = sorted(held)[0]
                    pid, cmd = held[port]
                    text = f"port {port} held by pid {pid if pid is not None else '?'} ({cmd})"
                    return self.finish(check, [Record(id=check.id, status="SKIPPED", reason=text)], paths=None,
                                       seconds=None)
            paths = Paths.under(self.run_dir, check.id)
            before = hygiene.process_keys()
            tree_before = hygiene.tree_state(self.cfg.root, self.run_id)

            def started(pid: int) -> None:
                self._live[check.id] = pid

            outcome = run_check(check, root=self.cfg.root, paths=paths, run_id=self.run_id, run_dir=self.run_dir,
                                kill_grace_s=self.cfg.kill_grace_s, on_start=started, start_lock=self._live_lock)
            with self._live_lock:
                self._live.pop(check.id, None)
                found = hygiene.find_leftovers(runner_pid=self.runner_pid, run_id=self.run_id, finished=check.id,
                                               scratch=paths.scratch, live=dict(self._live), before=before)
                hygiene.kill_leftovers(found, self.runner_pid)
            tree_after = hygiene.tree_state(self.cfg.root, self.run_id)
        recs = self.parse(check, adapter, paths, outcome)
        by_owner: dict[str, list[hygiene.Leftover]] = {}
        for left in found:
            by_owner.setdefault(left.owner, []).append(left)
        for owner, lefts in by_owner.items():
            text = "left running: " + "; ".join(lo.describe() for lo in lefts)
            rec = Record(id=f"hygiene/{owner}", status="FAIL", reason=text, assertion=record.clip(text), seams=[],
                         check=owner if self.cfg.seam_map.check(owner) else check.id)
            recs.append(rec)
        changed = hygiene.tree_diff(tree_before, tree_after)
        if changed:
            names = ", ".join(changed[:TREE_NAMES_MAX])
            if len(changed) > TREE_NAMES_MAX:
                names += f" (+{len(changed) - TREE_NAMES_MAX} more)"
            with self._live_lock:
                others = sorted(self._live)
            text = f"changed the tree during the check: {names}"
            if others:
                text += f" (concurrent checks: {', '.join(others)})"
            recs.append(Record(id=f"hygiene/{check.id}/tree", status="FAIL", reason=text,
                               assertion=record.clip(text), seams=[]))
        return self.finish(check, recs, paths=paths, seconds=outcome.duration_s)

    def parse(self, check: Check, adapter: adapters.Adapter, paths: Paths, outcome: Any) -> list[Record]:
        """The adapter's records plus the engine's two rules (timeout = FAIL; non-zero exit, nothing parsed = FAIL)."""
        ctx = AdapterContext(root=self.cfg.root, scratch=paths.scratch, tmp=paths.tmp, results=paths.results,
                             timed_out=outcome.timed_out, seam_map=self.cfg.seam_map, facts=dict(self.facts))
        try:
            recs = list(adapter(check, paths.log, outcome.status, ctx))
        except Exception as exc:  # an adapter bug is a FAIL of the check, never a green
            tail, line = adapters.log_tail(paths.log)
            recs = [Record(id=check.id, status="FAIL", reason=f"adapter {check.adapter!r} crashed: {exc!r}",
                           assertion=record.clip(tail) or None, log_line=line or None)]
        for rec in recs:
            if rec.status not in record.STATUSES:
                rec.reason = f"adapter gave status {rec.status!r}; " + (rec.reason or "")
                rec.status = "FAIL"
            if rec.status == "SKIPPED" and not rec.reason:
                rec.reason = "skipped (no reason given)"
        tail, line = adapters.log_tail(paths.log)
        if outcome.error is not None:
            recs.append(Record(id=check.id, status="FAIL", reason=f"could not start: {outcome.error}"))
        elif outcome.timed_out:
            how = f"timeout after {check.timeout_s} s (TERM to the group, then KILL after {self.cfg.kill_grace_s:g} s)"
            if outcome.killed:
                how += "; it needed the KILL"
            target = next((r for r in recs if r.id == check.id), None)
            if target is None:
                recs.append(Record(id=check.id, status="FAIL", reason=how, assertion=record.clip(tail) or None,
                                   log_line=line or None))
            else:
                target.status, target.reason = "FAIL", how
        elif outcome.status != 0 and not any(r.status in ("FAIL", "SKIPPED") for r in recs):
            recs = [r for r in recs if r.id != check.id]
            recs.append(Record(id=check.id, status="FAIL",
                               reason=f"exit {outcome.status} but no failing result was parsed",
                               assertion=record.clip(tail) or None, log_line=line or None))
        if not recs:
            recs.append(Record(id=check.id, status="FAIL", reason="exit 0 but the adapter returned no result",
                               assertion=record.clip(tail) or None, log_line=line or None))
        return recs


def execute(cfg: Config) -> Result:
    """Run ``cfg.plan`` (see the module docstring)."""
    return Engine(cfg).execute()
