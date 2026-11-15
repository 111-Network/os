"""One run: the scheduler that runs a plan's checks under the box rules and writes the records (options.md §3.2–3.3).

``execute(cfg)`` creates ``<scratch>/<run-id>/`` (run-id = UTC time plus the short HEAD), sweeps the leftovers of dead
runs (below), writes the ``run`` record, then runs the plan with ``cfg.jobs`` workers (default: the cargo slots, so 1).
For each check, in this order:

1. SKIPPED up front when a prerequisite did not pass (``requires <id> (<status>)``; a required unit whose covering gate
   is in the plan is decided by that gate, ``Plan.via``), under ``--offline`` for an ``offline_skip`` check
   (``offline``), or for a missing need (``needs <x> (missing)``);
2. its named locks, then a cargo slot when ``cargo = true`` (``t111.locks``). The scheduler starts a check only when
   this run holds none of the locks it needs (no named lock a running check of this run holds, a cargo slot free of
   this run's checks): it picks another startable check instead, so a check never waits on its own run. A wait on
   another run prints one line naming the holder (``waiting for <lock> held by run <id> (pid <n>)``, under ``--ci``
   too: it is a diagnosis) and is bounded: after ``--lock-wait`` s the check FAILs ``lock <name> not free after N s
   (held by run <id>, pid <n>)``. The held lock fds are handed to the check, so a lock lives as long as any process of
   the check (even when this runner dies);
3. for an ``ns`` check, the port pre-check (``port 11180 held by pid N (<cmdline>)`` = SKIPPED);
4. the process and tree snapshots, the subprocess session (``t111.run``; its output capped at ``--log-cap`` MB), the
   leftover scan (``hygiene/<check>``, killed) and the tree diff (``hygiene/<check>/tree``, naming every check that
   ran in that window);
5. the adapter, then the engine's rules: a check that could not start is one FAIL; a check the runner stopped is the
   interrupt FAIL whatever its exit status; a timeout is a FAIL; a non-zero exit with no FAIL record is a FAIL unless
   the adapter declared that exit (``AdapterContext.exit_declared``); then the map enrichment, the ``[[known]]`` rule
   (adapter-parsed results only), the non-zero-exit rule once more (a known FAIL turned SKIPPED never hides the exit),
   ``--require`` and the cascade link ``after`` (over the records of this harness run that are still FAIL);
6. the check's scratch is deleted when it has no FAIL (unless ``--keep``); a red one is kept and named in its records.

Records stream to ``results.jsonl`` as checks finish; ``summary.txt`` and stdout get the summary in plan order; a log
cut at ``--log-cap`` adds one ``note:`` line to the summary.

**Stopping.** SIGINT, SIGTERM and SIGHUP (a signal the parent set to ignore stays ignored) stop the run the same way.
``t111.cli.main`` installs the handlers first thing (``StopState``), so a signal during the map load, the selection or
the set-up is noted too, and one that arrives after the checks but before the report still makes the run RED:
no new check starts, the running checks' process groups get SIGTERM and, after the kill grace, SIGKILL; then the
leftover scan kills everything proven this run's (the run marker, the runner's descendants, the run dir in a command
line or environment: ``hygiene.sweep_run``; one that only names the run dir is reported, never killed), the records
that can be written are written (a killed check, whatever its exit status: one FAIL
``interrupted by SIGTERM: the runner killed the check``; one not reached: SKIPPED ``not started``), one FAIL record
``runner`` says why, and the run ends ``VERIFY RED`` with exit 1: an interrupted run is never green. An internal error
of the runner takes the same path with the FAIL ``runner error: <type>: <text>`` (and its traceback on stderr): whatever
happens, the last line printed is a ``VERIFY`` line, and no write to stdout or stderr raises (a closed pipe).

**Dead runs.** Every check carries ``T111_RUNNER_PID``/``T111_RUNNER_START`` and ``T111_LOCKS``. At its start a run
kills (TERM, then KILL) every process whose environ carries ``T111_RUN`` of another run whose runner is gone and that
shares this run's lock dir (``hygiene.sweep_dead_runs``)
and prints one line ``note: killed N leftover process(es) of dead run <id>`` (also in the summary; a note, not a FAIL).
A process without the marker is never touched.

**Attribution (``--shared-box``).** By default (a quiet tree: CI, a lead's run after its workers returned) a tree
change during a check and a look-alike process (our binary, no marker) are FAIL. With ``--shared-box`` (or
``T111_SHARED_BOX=1``), for a run beside other writers, both are SKIPPED ``not attributed (shared box): ...``: the run
never ends RED on something it cannot prove. A process proven ours (the marker, a descendant, the scratch dir) stays
FAIL in both modes; no tree path is ever "proven" (a path only this check could have written is a guess). A process
that only names the check's scratch dir (an agent's ``tail -f`` on a log) is never proven: strict, a FAIL saying
``[not killed: …]``; ``--shared-box``, SKIPPED; never killed.
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
import traceback
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, TextIO

from . import adapters, hygiene, locks as locks_mod, record, summary
from .adapters import AdapterContext
from .record import Record, RerunPaths
from .run import KILL_GRACE_S, LOG_CAP_MB, Outcome, Paths, run_check
from .schema import Check, SeamMap
from .select import Plan

DEFAULT_TOOLS: tuple[str, ...] = ("cargo", "pnpm", "node", "python3", "git", "shellcheck", "unshare", "nsenter", "ip",
                                  "curl", "jq", "ss")
"""Tools the run record always reports (plus every tool a selected check needs)."""

TREE_NAMES_MAX = 20
"""How many changed paths a tree-hygiene record names before ``+N more``."""

STOP_SIGNALS: tuple[int, ...] = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
"""The signals that stop a run (each like Ctrl-C)."""

STOP_JOIN_S = 30.0
"""How long a stopping run waits for its workers after the SIGKILL before it reports without them."""

NOT_ATTRIBUTED = "not attributed (shared box)"
"""The reason prefix of a SKIPPED hygiene record under ``--shared-box``."""

RUNNER_ID = "runner"
"""The id of the record that says why a run stopped (an interrupt or a runner error)."""


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
    shared_box: bool = False
    lock_wait_s: float = locks_mod.LOCK_WAIT_S
    log_cap_mb: float = LOG_CAP_MB
    kill_grace_s: float = KILL_GRACE_S
    ns_ports: tuple[int, ...] = hygiene.NS_PORTS
    lock_dir: Path | None = None
    argv: list[str] = field(default_factory=list)
    out: TextIO | None = None
    err: TextIO | None = None
    stop_state: "StopState | None" = None


@dataclass
class Result:
    """What ``execute`` returns: the run dir, every record in plan order, the summary text and the exit code."""

    run_dir: Path
    records: list[Record]
    text: str
    exit_code: int


@dataclass
class Ran:
    """What happened to one started check inside its locks."""

    paths: Paths
    outcome: Outcome
    found: list[hygiene.Leftover]
    changed: list[str]
    window: list[str]
    cut: bool


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
    head = done.stdout.decode("utf-8", "replace").strip()
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


def emit(stream: TextIO, text: str) -> None:
    """Write ``text`` to ``stream`` and flush; never raises (an encoding falls back to ASCII escapes; a closed or
    broken stream, an agent's ``| head``, is ignored: stdout and stderr are advisory, the records are the file)."""
    try:
        try:
            stream.write(text)
        except UnicodeEncodeError:
            stream.write(text.encode("ascii", "backslashreplace").decode("ascii"))
        stream.flush()
    except (OSError, ValueError):
        pass


class StopState:
    """The stop signals of one process (SIGINT, SIGTERM, SIGHUP): the first one is noted (``name``), never acted on in
    the handler. ``t111.cli.main`` installs it before anything else; the engine reads it (and installs it itself when
    run in process without the cli). A signal the parent set to ignore stays ignored."""

    def __init__(self) -> None:
        self.name: str | None = None
        self._old: dict[int, Any] = {}

    @property
    def installed(self) -> bool:
        return bool(self._old)

    def handler(self, signum: int, _frame: Any) -> None:
        if self.name is None:
            self.name = signal.Signals(signum).name

    def install(self) -> None:
        if threading.current_thread() is not threading.main_thread():
            return
        for sig in STOP_SIGNALS:
            try:
                if signal.getsignal(sig) == signal.SIG_IGN:
                    continue
                self._old[sig] = signal.signal(sig, self.handler)
            except (ValueError, OSError):
                continue

    def restore(self) -> None:
        for sig, old in self._old.items():
            try:
                signal.signal(sig, old)
            except (ValueError, OSError, TypeError):
                pass
        self._old.clear()


def runner_fail(reason: str) -> Record:
    """The ``runner`` FAIL record (an interrupt or a runner error); engine-made, tied to no seam."""
    return Record(id=RUNNER_ID, status="FAIL", reason=reason, seams=[], engine=True)


def error_words(exc: BaseException) -> str:
    """``runner error: <type>: <text>``."""
    return f"runner error: {type(exc).__name__}: {exc}"


class Engine:
    """Runs one plan. Shared state (statuses, records, the live checks) is guarded by ``_cv`` and ``_live_lock``;
    ``_live_lock`` is ``hygiene.SPAWNED.lock``, the lock under which every pid ``subprocess`` owns is registered, so
    no reap and no leftover scan ever sees a process between its fork and its registration."""

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
        self.locks = locks_mod.Locks(cfg.lock_dir or locks_mod.lock_dir(), cfg.cargo_slots, run_id=self.run_id,
                                     pid=self.runner_pid, wait_s=cfg.lock_wait_s)
        self.probes = hygiene.Probes(offline=cfg.offline, run_id=self.run_id, cwd=cfg.root)
        self.facts = {"euid0": os.geteuid() == 0, "offline": cfg.offline}
        self.writer = record.ResultsWriter(self.run_dir / "results.jsonl")
        self._cv = threading.Condition()
        self._live_lock = hygiene.SPAWNED.lock
        self._live: dict[str, int] = {}
        self._windows: dict[str, set[str]] = {}
        self._began: set[str] = set()
        self._status: dict[str, str] = {}
        self._records: dict[str, list[Record]] = {}
        self._pending: list[Check] = list(cfg.plan.checks)
        self._failed: BaseException | None = None
        self._stop = threading.Event()
        self._stop_why = ""
        self._stopped_live: set[str] = set()
        self.stop_state = cfg.stop_state or StopState()
        self._own_handlers = False
        self._held_names: dict[str, set[str]] = {}
        self._held_cargo: set[str] = set()
        self.notes: list[str] = []
        self.swept: dict[str, int] = {}

    # -- the run record ------------------------------------------------------------------------------------------

    def run_record(self) -> dict[str, Any]:
        """The first line of ``results.jsonl``: what the box had, so every skip reason can be checked."""
        plan = self.cfg.plan.checks
        needs = sorted({n for c in plan for n in c.needs if n not in hygiene.Probes.NAMED})
        tools = {t: shutil.which(t) is not None for t in sorted(set(DEFAULT_TOOLS) | set(needs))}
        want_ns = any(c.tier == "ns" or {"userns", "netns"} & set(c.needs) for c in plan)
        tree = hygiene.tree_state(self.cfg.root, self.run_id, self.cfg.scratch)
        return {
            "v": record.RECORD_VERSION, "kind": "run", "run_id": self.run_id, "head": self.head,
            "dirty": None if tree is None else bool(tree), "euid": os.geteuid(), "tools": tools,
            "namespaces": self.probes.has("userns") if want_ns else None, "subreaper": self.subreaper,
            "argv": self.cfg.argv, "tiers": list(self.cfg.plan.tiers), "checks": [c.id for c in plan],
            "pulled": sorted(self.cfg.plan.pulled), "cargo_slots": self.cfg.cargo_slots, "jobs": self.jobs,
            "offline": self.cfg.offline, "require": self.cfg.require, "ci": self.cfg.ci, "keep": self.cfg.keep,
            "root": str(self.cfg.root), "run_dir": str(self.run_dir), "started": _dt.datetime.now(
                _dt.timezone.utc).isoformat(timespec="seconds"), "python": sys.version.split()[0],
            "shared_box": self.cfg.shared_box, "lock_wait_s": self.cfg.lock_wait_s,
            "log_cap_mb": self.cfg.log_cap_mb, "via": dict(self.cfg.plan.via), "swept": dict(self.swept),
        }

    # -- signals -------------------------------------------------------------------------------------------------

    @property
    def _interrupt(self) -> str | None:
        """The first stop signal seen (by the cli's handlers or the engine's own), or None."""
        return self.stop_state.name

    def _install_signals(self) -> None:
        if not self.stop_state.installed:
            self.stop_state.install()
            self._own_handlers = True

    def _restore_signals(self) -> None:
        if self._own_handlers:
            self.stop_state.restore()
            self._own_handlers = False

    # -- scheduling ----------------------------------------------------------------------------------------------

    def _next(self) -> Check | None:
        with self._cv:
            while True:
                if self._failed is not None or self._stop.is_set() or not self._pending:
                    return None
                plan_ids = {c.id for c in self.cfg.plan.checks}
                for index, check in enumerate(self._pending):
                    needed = [self.cfg.plan.requirement(r) for r in check.requires]
                    if all(r in self._status or r not in plan_ids for r in needed) and self._free_here(check):
                        del self._pending[index]
                        self._held_names[check.id] = set(check.locks())
                        if check.cargo:
                            self._held_cargo.add(check.id)
                        return check
                self._cv.wait(0.5)

    def _free_here(self, check: Check) -> bool:
        """True when no running check of this run holds a lock ``check`` needs (call under ``_cv``): a named lock
        another of our checks holds, or every cargo slot taken by our checks, makes the scheduler pick another."""
        names = set(check.locks())
        if any(names & held for held in self._held_names.values()):
            return False
        return not (check.cargo and len(self._held_cargo) >= self.cfg.cargo_slots)

    def _release_here(self, check: Check) -> None:
        """Forget a finished check's locks (call under ``_cv``)."""
        self._held_names.pop(check.id, None)
        self._held_cargo.discard(check.id)

    def _worker(self) -> None:
        try:
            while True:
                check = self._next()
                if check is None:
                    return
                try:
                    done = self.process(check)
                finally:
                    with self._cv:
                        self._release_here(check)
                        self._cv.notify_all()
                with self._cv:
                    if done is not None:
                        self._status[check.id], self._records[check.id] = done
                    self._cv.notify_all()
        except BaseException as exc:  # a runner bug must not hang the other workers
            with self._cv:
                self._failed = exc
                self._cv.notify_all()

    def _alive(self, workers: list[threading.Thread]) -> bool:
        return any(w.is_alive() for w in workers)

    def _join(self, workers: list[threading.Thread], deadline: float | None = None) -> None:
        """Wait for the workers; return early on a stop signal or a worker failure (or at ``deadline``)."""
        while self._alive(workers):
            if deadline is None and (self._interrupt is not None or self._failed is not None):
                return
            if deadline is not None and time.monotonic() >= deadline:
                return
            alive = next((w for w in workers if w.is_alive()), None)
            if alive is not None:
                alive.join(0.1)

    def sweep_dead(self) -> None:
        """Kill the marked leftovers of dead runs (module docstring); one note line per dead run."""
        for other, procs in sorted(hygiene.sweep_dead_runs(self.run_id, lock_dir=self.locks.directory).items()):
            killed = hygiene.terminate(procs, runner_pid=self.runner_pid)
            if not killed:
                continue
            self.swept[other] = killed
            note = f"note: killed {killed} leftover process(es) of dead run {other}"
            self.notes.append(note)
            if not self.cfg.ci:
                emit(self.err, f"t111: {note}\n")

    def execute(self) -> Result:
        """Run the plan; return the records, the summary and the exit code. Never raises (an internal error is a FAIL
        record ``runner`` and the run ends ``VERIFY RED``)."""
        began = time.monotonic()
        extra: list[Record] = []
        workers: list[threading.Thread] = []
        try:
            self._install_signals()
            try:
                self.sweep_dead()
                self.writer.write(self.run_record())
                workers = [threading.Thread(target=self._worker, name=f"t111-worker-{i}", daemon=True)
                           for i in range(max(1, self.jobs))]
                for worker in workers:
                    worker.start()
                self._join(workers)
            except Exception as exc:
                emit(self.err, "".join(traceback.format_exception(exc)))
                extra.append(runner_fail(error_words(exc)))
            if self._failed is not None:
                emit(self.err, "".join(traceback.format_exception(self._failed)))
                extra.append(runner_fail(error_words(self._failed)))
            if self._interrupt is not None or extra:
                why = f"interrupted by {self._interrupt}" if self._interrupt is not None else "stopped by a runner error"
                extra = self.stop(workers, why) + extra
            else:
                hygiene.reap_orphans(self.runner_pid)
                if self._interrupt is not None:  # a signal while the orphans were reaped: still never green
                    extra = self.stop(workers, f"interrupted by {self._interrupt}")
            return self.report(began, extra)
        except Exception as exc:  # the report itself failed: still a last line
            return self.last_resort(exc)
        finally:
            self._restore_signals()
            self.writer.close()

    def stop(self, workers: list[threading.Thread], why: str) -> list[Record]:
        """Stop the run (module docstring): kill the running checks, sweep what is proven ours, and return the records
        of the stop itself (the sweep's leftovers, the ``runner`` record of an interrupt). Records of checks never
        reached are written here too."""
        self._stop_why = why
        self._stop.set()
        with self._cv:
            self._cv.notify_all()
        with self._live_lock:
            self._stopped_live = set(self._live)
            self._signal_live(signal.SIGTERM)
        self._join(workers, time.monotonic() + self.cfg.kill_grace_s)
        with self._live_lock:
            self._signal_live(signal.SIGKILL)
        self._join(workers, time.monotonic() + STOP_JOIN_S)
        out: list[Record] = []
        left = hygiene.sweep_run(runner_pid=self.runner_pid, run_id=self.run_id, run_dir=self.run_dir)
        hygiene.terminate([lo.proc for lo in left if lo.kill], runner_pid=self.runner_pid)
        hygiene.reap_orphans(self.runner_pid)
        shown = [lo for lo in left if lo.kill or not self.cfg.shared_box]
        unproven = [lo for lo in left if not lo.kill and self.cfg.shared_box]
        if shown:
            text = "left running when the run stopped (killed unless marked): " + "; ".join(
                f"{lo.describe()} [{lo.owner}, {lo.rule}]" for lo in shown)
            out.append(Record(id="hygiene/run", status="FAIL", reason=text, assertion=record.clip(text), seams=[],
                              engine=True))
        if unproven:
            text = f"{NOT_ATTRIBUTED}: " + "; ".join(lo.describe() for lo in unproven)
            out.append(Record(id="hygiene/run/unattributed", status="SKIPPED", reason=text, seams=[], engine=True))
        with self._cv:
            done = {cid: list(recs) for cid, recs in self._records.items()}
        with self._live_lock:
            began = set(self._began)
        killed = not_started = 0
        for check in self.cfg.plan.checks:
            if check.id in done:
                killed += any(r.id == check.id and (r.reason or "").startswith(why) for r in done[check.id])
                continue
            if check.id in began:
                killed += 1
                rec = Record(id=check.id, status="FAIL", engine=True,
                             reason=f"{why}: the check's records were lost (its worker did not finish)")
            else:
                not_started += 1
                rec = Record(id=check.id, status="SKIPPED", reason=f"not started: {why}", engine=True)
            record.enrich(rec, check, self.cfg.seam_map, root=self.cfg.root, where=self.rerun_paths(None))
            self._write(rec)
            with self._cv:
                self._records[check.id] = [rec]
        if self._interrupt is not None:
            out.append(runner_fail(f"{why}: {killed} check(s) killed, {not_started} not started"))
        return out

    def _write(self, rec: Record) -> None:
        """Write one record during a stop; a writer that fails is reported on stderr, never raised (F6)."""
        try:
            self.writer.write(rec.to_json())
        except Exception as exc:  # noqa: BLE001 - the summary must still be printed
            emit(self.err, f"t111: could not write the record {rec.id}: {type(exc).__name__}: {exc}\n")

    def _signal_live(self, sig: int) -> None:
        """Signal the process group of every check still running (call under ``_live_lock``)."""
        for pid in self._live.values():
            try:
                os.killpg(pid, sig)
            except (ProcessLookupError, PermissionError):
                pass

    def report(self, began: float, extra: list[Record]) -> Result:
        """Write the stop's records, ``summary.txt`` and stdout; return the result."""
        for rec in extra:
            self._write(rec)
        with self._cv:
            records = [r for c in self.cfg.plan.checks for r in self._records.get(c.id, [])] + extra
        results = self.run_dir / "results.jsonl"
        parity_none = sum(1 for f in self.cfg.seam_map.fakes if f.parity == "none")
        text, code = summary.render(records, n_checks=len(self.cfg.plan.checks), tiers=self.cfg.plan.tiers,
                                    duration_s=time.monotonic() - began, results=str(results),
                                    parity_none=parity_none, notes=self.notes)
        try:
            (self.run_dir / "summary.txt").write_text(text, encoding="utf-8", errors="backslashreplace")
        except OSError as exc:
            emit(self.err, f"t111: could not write summary.txt: {exc}\n")
        emit(self.out, text)
        return Result(self.run_dir, records, text, code)

    def last_resort(self, exc: BaseException) -> Result:
        """The report failed: print a minimal summary that still ends ``VERIFY RED``."""
        emit(self.err, "".join(traceback.format_exception(exc)))
        rec = runner_fail(error_words(exc))
        text = record.safe_text(f"verify: runner error while reporting — {self.run_dir}\n"
                                f"FAIL - [-] {RUNNER_ID}: {rec.reason}\nVERIFY RED (1 FAIL)\n")
        try:
            emit(self.out, text)
        except Exception:  # noqa: BLE001 - nothing left to report to
            pass
        return Result(self.run_dir, [rec], text, summary.EXIT_RED)

    def progress(self, check: Check, status: str, recs: list[Record], seconds: float | None) -> None:
        if self.cfg.ci:
            return
        counts = {s: sum(1 for r in recs if r.status == s) for s in record.STATUSES}
        detail = ", ".join(f"{n} {s}" for s, n in counts.items() if n)
        took = f" in {summary.duration(seconds)}" if seconds is not None else ""
        emit(self.err, f"t111: {check.id} {status} ({detail}){took}\n")

    # -- one check -----------------------------------------------------------------------------------------------

    def skip_reason(self, check: Check) -> str | None:
        """Why ``check`` is SKIPPED before it starts, or None."""
        with self._cv:
            for req in check.requires:
                status = self._status.get(self.cfg.plan.requirement(req))
                if status is not None and status != "PASS":
                    via = self.cfg.plan.requirement(req)
                    return f"requires {req} ({status})" if via == req else f"requires {req} (via {via}: {status})"
        if self.cfg.offline and check.offline_skip:
            return "offline"
        missing = self.probes.missing(check.needs)
        if missing:
            return f"needs {', '.join(missing)} (missing)"
        return None

    def rerun_paths(self, paths: Paths | None) -> RerunPaths:
        """Where a record's rerun points ``{scratch}``/``{tmp}``: the kept scratch under ``--keep``, else fresh."""
        kept = paths.scratch if (self.cfg.keep and paths is not None) else None
        return RerunPaths(root=self.cfg.root, run=self.run_dir, scratch=kept)

    def finish(self, check: Check, recs: list[Record], *, paths: Paths | None, seconds: float | None,
               exit_status: int | None = None, declared: bool = False) -> tuple[str, list[Record]]:
        """Enrich, apply the known rule, the non-zero-exit rule again (``exit_status`` of a check that ran, unless the
        adapter ``declared`` it), the require and after rules, write; return the check's status and its records."""
        where = self.rerun_paths(paths)
        for rec in recs:
            if rec.seams is None and rec.id.startswith("hygiene/"):
                rec.seams = []
            record.enrich(rec, check, self.cfg.seam_map, root=self.cfg.root, where=where)
            rec.duration_s = None if seconds is None else round(seconds, 3)
        for rec in recs:
            record.apply_known(rec, self.cfg.seam_map.known, self.facts)
        if exit_status not in (None, 0) and not declared and paths is not None and not any(
                r.status == "FAIL" and not r.id.startswith("hygiene/") for r in recs):
            tail, line = adapters.log_tail(paths.log)
            rec = Record(id=check.id, status="FAIL", reason=f"exit {exit_status} but no failing result was parsed",
                         assertion=tail or None, log_line=line or None, engine=True)
            record.enrich(rec, check, self.cfg.seam_map, root=self.cfg.root, where=where)
            rec.duration_s = None if seconds is None else round(seconds, 3)
            recs = replace_own(recs, check.id, rec)
        if self.cfg.require:
            for rec in recs:
                record.apply_require(rec)
        record.mark_after(recs, one_harness=True)
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

    def made(self, check: Check, status: str, reason: str) -> list[Record]:
        """One engine-made record of ``check``."""
        return [Record(id=check.id, status=status, reason=reason, engine=True)]

    def process(self, check: Check) -> tuple[str, list[Record]] | None:
        """Run one check through steps 1–6 of the module docstring; None when the run stopped before it started."""
        reason = self.skip_reason(check)
        if reason is not None:
            return self.finish(check, self.made(check, "SKIPPED", reason), paths=None, seconds=None)
        adapter = adapters.get(check.adapter)
        if adapter is None:
            return self.finish(check, self.made(check, "FAIL", f"unknown adapter {check.adapter!r} (registered: "
                                                f"{', '.join(sorted(adapters.REGISTRY))})"), paths=None, seconds=None)

        def waiting(text: str) -> None:  # a diagnosis, not progress: printed under --ci too
            emit(self.err, f"t111: {check.id} {text}\n")

        try:
            with self.locks.for_check(check.locks(), check.cargo, cancel=self._stop, on_wait=waiting) as held:
                ran = self.run_locked(check, held.fds)
        except locks_mod.LockTimeout as exc:
            return self.finish(check, self.made(check, "FAIL", str(exc)), paths=None, seconds=None)
        except locks_mod.LockCancelled:
            return None
        if ran is None:
            return None
        if isinstance(ran, str):
            return self.finish(check, self.made(check, "SKIPPED", ran), paths=None, seconds=None)
        recs, declared = self.parse_declared(check, adapter, ran)
        recs += self.hygiene_records(check, ran)
        status = None if (ran.outcome.timed_out or check.id in self._stopped_live) else ran.outcome.status
        return self.finish(check, recs, paths=ran.paths, seconds=ran.outcome.duration_s, exit_status=status,
                           declared=declared)

    def run_locked(self, check: Check, fds: list[int]) -> Ran | str | None:
        """Steps 3–4 under the check's locks: a skip reason (the port pre-check), None (the run stopped first), or
        what ran."""
        if check.tier == "ns":
            held = hygiene.port_holders(self.cfg.ns_ports)
            if held:
                port = sorted(held)[0]
                pid, cmd = held[port]
                return f"port {port} held by pid {pid if pid is not None else '?'} ({cmd})"
        paths = Paths.under(self.run_dir, check.id)
        with self._live_lock:
            self._windows[check.id] = set(self._live) | {check.id}
        before = hygiene.process_keys()
        tree_before = hygiene.tree_state(self.cfg.root, self.run_id, self.cfg.scratch)

        def started(pid: int) -> None:  # under _live_lock (run_check holds it across the fork)
            self._live[check.id] = pid
            self._began.add(check.id)
            for window in self._windows.values():
                window.add(check.id)

        outcome = run_check(check, root=self.cfg.root, paths=paths, run_id=self.run_id, run_dir=self.run_dir,
                            kill_grace_s=self.cfg.kill_grace_s, on_start=started, start_lock=self._live_lock,
                            pass_fds=fds, log_cap_mb=self.cfg.log_cap_mb, drain_later=True,
                            cancelled=self._stop.is_set, lock_dir=self.locks.directory)
        if outcome.not_started:
            with self._live_lock:
                self._windows.pop(check.id, None)
            shutil.rmtree(paths.scratch, ignore_errors=True)
            return None
        with self._live_lock:
            self._live.pop(check.id, None)
            found = hygiene.find_leftovers(runner_pid=self.runner_pid, run_id=self.run_id, finished=check.id,
                                           scratch=paths.scratch, live=dict(self._live), before=before)
            hygiene.kill_leftovers(found, self.runner_pid)
        cut = outcome.finish_log()
        if cut and outcome.pump is not None:
            with self._cv:
                self.notes.append(f"note: {check.id}: {outcome.pump.describe()}")
        tree_after = hygiene.tree_state(self.cfg.root, self.run_id, self.cfg.scratch)
        with self._live_lock:
            window = sorted(self._windows.pop(check.id, {check.id}))
        return Ran(paths, outcome, found, hygiene.tree_diff(tree_before, tree_after), window, cut)

    def hygiene_records(self, check: Check, ran: Ran) -> list[Record]:
        """The leftover and tree records of one check (strict, or ``--shared-box``: module docstring)."""
        out: list[Record] = []
        by_owner: dict[str, list[hygiene.Leftover]] = {}
        unproven: list[hygiene.Leftover] = []
        for left in ran.found:
            if left.kill or not self.cfg.shared_box:
                by_owner.setdefault(left.owner, []).append(left)
            else:
                unproven.append(left)
        for owner, lefts in by_owner.items():
            text = "left running: " + "; ".join(lo.describe() for lo in lefts)
            out.append(Record(id=f"hygiene/{owner}", status="FAIL", reason=text, assertion=record.clip(text), seams=[],
                              check=owner if self.cfg.seam_map.check(owner) else check.id, engine=True))
        if unproven:
            text = f"{NOT_ATTRIBUTED}: " + "; ".join(lo.describe() for lo in unproven)
            out.append(Record(id=f"hygiene/{check.id}/unattributed", status="SKIPPED", reason=text, seams=[],
                              engine=True))
        if ran.changed:
            names = ", ".join(ran.changed[:TREE_NAMES_MAX])
            if len(ran.changed) > TREE_NAMES_MAX:
                names += f" (+{len(ran.changed) - TREE_NAMES_MAX} more)"
            text = f"the tree changed during the check: {names} (checks running in that window: {', '.join(ran.window)})"
            if self.cfg.shared_box:
                out.append(Record(id=f"hygiene/{check.id}/tree", status="SKIPPED", reason=f"{NOT_ATTRIBUTED}: {text}",
                                  seams=[], engine=True))
            else:
                out.append(Record(id=f"hygiene/{check.id}/tree", status="FAIL", reason=text,
                                  assertion=record.clip(text), seams=[], engine=True))
        return out

    def parse(self, check: Check, adapter: adapters.Adapter, ran: Ran) -> list[Record]:
        """The adapter's records plus the engine's rules (module docstring, step 5); one record per check id."""
        return self.parse_declared(check, adapter, ran)[0]

    def parse_declared(self, check: Check, adapter: adapters.Adapter, ran: Ran) -> tuple[list[Record], bool]:
        """``parse``, and whether the adapter declared the exit (``AdapterContext.exit_declared``)."""
        paths, outcome = ran.paths, ran.outcome
        cache: dict[str, tuple[str, int]] = {}

        def fail(reason: str) -> Record:
            if "tail" not in cache:
                cache["tail"] = adapters.log_tail(paths.log)
            text, line = cache["tail"]
            return Record(id=check.id, status="FAIL", reason=reason, assertion=text or None, log_line=line or None,
                          engine=True)

        def replace(recs: list[Record], new: Record) -> list[Record]:
            return replace_own(recs, check.id, new)

        if outcome.error is not None:
            return [fail(f"could not start: {outcome.error}")], False
        ctx = AdapterContext(root=self.cfg.root, scratch=paths.scratch, tmp=paths.tmp, results=paths.results,
                             timed_out=outcome.timed_out, seam_map=self.cfg.seam_map, facts=dict(self.facts))
        try:
            recs = list(adapter(check, paths.log, outcome.status, ctx))
        except Exception as exc:  # an adapter bug is a FAIL of the check, never a green
            recs = [fail(f"adapter {check.adapter!r} crashed: {exc!r}")]
        for rec in recs:
            if rec.status not in record.STATUSES:
                rec.reason = f"adapter gave status {rec.status!r}; " + (rec.reason or "")
                rec.status = "FAIL"
                rec.engine = True
            if rec.status == "SKIPPED" and not rec.reason:
                rec.reason = "skipped (no reason given)"
        if check.id in self._stopped_live:  # stopped: never PASS, whatever a TERM trap made the exit status
            recs = replace(recs, fail(f"{self._stop_why}: the runner killed the check"))
        elif outcome.timed_out:
            how = f"timeout after {check.timeout_s} s (TERM to the group, then KILL after {self.cfg.kill_grace_s:g} s)"
            if outcome.killed:
                how += "; it needed the KILL"
            old = next((r for r in recs if r.id == check.id), None)
            new = fail(how)
            if old is not None and old.assertion:
                new.assertion, new.log_line = old.assertion, old.log_line
            recs = replace(recs, new)
        elif outcome.status != 0 and not any(r.status == "FAIL" for r in recs) and not ctx.exit_declared:
            recs = replace(recs, fail(f"exit {outcome.status} but no failing result was parsed"))
        if not recs:
            recs.append(fail(f"exit {outcome.status} but the adapter returned no result"))
        if ran.cut:
            target = next((r for r in recs if r.id == check.id), recs[0])
            words = f"log cut at {self.cfg.log_cap_mb:g} MB"
            target.reason = f"{target.reason}; {words}" if target.reason else words
        return recs, ctx.exit_declared


def replace_own(recs: list[Record], check_id: str, new: Record) -> list[Record]:
    """``new`` in place of the check's own record (where the first one stood), never two with one id."""
    out: list[Record] = []
    for rec in recs:
        if rec.id == check_id:
            if new not in out:
                out.append(new)
        else:
            out.append(rec)
    if new not in out:
        out.append(new)
    return out


def failed_start(cfg: Config, exc: BaseException) -> Result:
    """The engine could not even set up the run: print the runner FAIL and ``VERIFY RED``."""
    err = cfg.err or sys.stderr
    emit(err, "".join(traceback.format_exception(exc)))
    rec = runner_fail(error_words(exc))
    text, code = summary.render([rec], n_checks=len(cfg.plan.checks), tiers=cfg.plan.tiers, duration_s=0,
                                results="(no run dir)")
    emit(cfg.out or sys.stdout, text)
    return Result(cfg.scratch, [rec], text, code)


def execute(cfg: Config) -> Result:
    """Run ``cfg.plan`` (see the module docstring)."""
    try:
        eng = Engine(cfg)
    except Exception as exc:
        return failed_start(cfg, exc)
    return eng.execute()


def interrupted_before_start(out: TextIO | None, name: str) -> str:
    """Print (and return) the summary of a run a stop signal ended before it was set up: no check ran, still RED."""
    text, _code = summary.render([runner_fail(f"interrupted by {name} before the run started")], n_checks=0,
                                 tiers=(), duration_s=0, results="(no run dir)")
    emit(out or sys.stdout, text)
    return text

