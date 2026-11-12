"""One check = one subprocess session (options.md §3.2).

``run_check`` starts the check's argv in its own session (``start_new_session``), from its cwd under the repo root,
with stdout and stderr on one pipe (one combined, ordered stream), stdin from ``/dev/null``, and the environment the
check declares on top of the runner's, plus:

- ``TMPDIR=<scratch>/tmp`` (every harness ``mktemp`` lands inside the check's scratch),
- ``T111_SCRATCH=<scratch>``, ``T111_RUN=<run-id>``, ``T111_CHECK=<check id>`` (the hygiene markers),
- ``T111_RUNNER_PID=<pid>``, ``T111_RUNNER_START=<start ticks>`` (the runner, so a later run can tell a dead run's
  leftovers from a live run's processes: ``t111.hygiene.sweep_dead_runs``),
- ``T111_RESULTS=<scratch>/emit.jsonl`` (the shared emitter's output file, F3 phase 2),
- ``T111_LOCKS=<lock dir>`` (the runner's lock dir: a nested runner shares the box's locks, and the dead-run sweep
  of ``t111.hygiene`` only touches the processes of runs that share its lock dir).

The pipe is copied into ``out.log`` by a ``LogPump`` thread: the head up to a cap (``--log-cap``, 64 MB), then the
pump keeps reading (the check never blocks on a full pipe) and keeps only the last 1 MB, where the adapters read the
summary and the last error. A cut log holds the head, one line ``t111: log cut at N MB: M bytes dropped here …``
and that last MB (from its first whole line).
The leader's pid is registered in ``t111.hygiene.SPAWNED`` in the same critical section as the fork; ``pass_fds`` hands
the check the lock fds it holds (``t111.locks``). At ``timeout_s`` the whole process group gets SIGTERM, then SIGKILL
after ``kill_grace_s`` (10 s). The outcome carries the exit status, whether it timed out, and the duration (for the
record only: no pass/fail decision reads it).
"""

from __future__ import annotations

import os
import select
import signal
import subprocess
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Callable, Mapping, Sequence

from . import hygiene
from .schema import Check

KILL_GRACE_S = 10.0
"""Seconds between SIGTERM and SIGKILL when a check times out."""

LOG_CAP_MB = 64.0
"""How many MB of a check's output ``out.log`` keeps (``--log-cap``)."""

TAIL_KEEP_BYTES = 1 << 20
"""How many closing bytes of a cut log ``LogPump`` keeps after the cut marker."""

DRAIN_MAX_READS = 256
"""At most this many 64 KB reads after ``LogPump.finish`` (16 MB): what was written before the stop is in the pipe."""

NOT_STARTED = "not started: the run is stopping"
"""``Outcome.error`` of a check the runner refused to start because the run was interrupted."""


@dataclass(frozen=True)
class Paths:
    """Where one check writes: its scratch dir, its TMPDIR, its log and its emitter file."""

    scratch: Path
    tmp: Path
    log: Path
    results: Path

    @classmethod
    def under(cls, run_dir: Path, check_id: str) -> "Paths":
        """The layout ``<run-dir>/<check id>/{tmp,out.log,emit.jsonl}``."""
        scratch = run_dir / check_id
        return cls(scratch, scratch / "tmp", scratch / "out.log", scratch / "emit.jsonl")


class LogPump:
    """Copies a check's output from the read end of its pipe into ``log``: the first ``cap_bytes`` bytes, and of the
    rest the last ``TAIL_KEEP_BYTES``.

    The thread reads until EOF (every writer gone) or until ``finish`` asks it to stop; then it drains what the pipe
    still holds without blocking and closes. Past the cap it keeps reading into a bounded tail buffer; at the close a
    cut log gets one marker line ``t111: log cut at N MB: M bytes dropped here (the head above, the last … below)``
    and then the tail (from its first whole line). ``cut`` says whether that happened, ``dropped`` how many bytes.
    """

    def __init__(self, fd: int, log: Path, cap_bytes: int, cap_mb: float):
        self._fd = fd
        self._log = log.open("ab", buffering=0)  # unbuffered: a reader of out.log sees the check live
        self.cap_bytes = max(0, cap_bytes)
        self.cap_mb = cap_mb
        self.written = 0
        self.cut = False
        self.dropped = 0
        self._tail = bytearray()
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, name="t111-log-pump", daemon=True)
        self._thread.start()

    def _write(self, data: bytes) -> None:
        """Write all of ``data`` to the unbuffered log (a raw write may take only part of it)."""
        view = memoryview(data)
        while view:
            n = self._log.write(view)
            if not n:  # nothing taken (a full disk): give up rather than spin
                return
            view = view[n:]

    def _keep(self, chunk: bytes) -> None:
        room = self.cap_bytes - self.written
        if room > 0:
            part = chunk[:room]
            self._write(part)
            self.written += len(part)
            chunk = chunk[room:]
        if chunk:
            self.cut = True
            self._tail += chunk
            over = len(self._tail) - TAIL_KEEP_BYTES
            if over > 0:
                del self._tail[:over]
                self.dropped += over

    def _drain(self) -> None:
        """Read what the pipe holds now, without blocking (bounded: a writer that never stops cannot hold us)."""
        os.set_blocking(self._fd, False)
        for _ in range(DRAIN_MAX_READS):
            try:
                chunk = os.read(self._fd, 1 << 16)
            except BlockingIOError:
                return
            if not chunk:
                return
            self._keep(chunk)

    def _run(self) -> None:
        try:
            while True:
                if self._stop.is_set():
                    self._drain()
                    return
                ready, _, _ = select.select([self._fd], [], [], 0.1)
                if not ready:
                    continue
                chunk = os.read(self._fd, 1 << 16)
                if not chunk:
                    return
                self._keep(chunk)
        finally:
            try:
                os.close(self._fd)
            except OSError:
                pass
            if self.cut:
                tail = bytes(self._tail)
                if self.dropped:
                    cut = tail.find(b"\n")
                    if 0 <= cut < len(tail) - 1:
                        self.dropped += cut + 1
                        tail = tail[cut + 1:]
                self._write(f"\nt111: log cut at {self.cap_mb:g} MB: {self.dropped} bytes dropped here (the "
                                f"head above, the last {len(tail)} bytes below)\n".encode())
                self._write(tail)
            self._log.close()

    def finish(self) -> bool:
        """Stop after draining what the pipe holds; wait for the thread; return ``cut``."""
        self._stop.set()
        self._thread.join()
        return self.cut

    def describe(self) -> str:
        """The note of a cut log (for the summary)."""
        return (f"log cut at {self.cap_mb:g} MB: {self.dropped} bytes dropped between the head and the last "
                f"{TAIL_KEEP_BYTES // (1 << 20)} MB")


@dataclass(frozen=True)
class Outcome:
    """What happened to the process: ``status`` (negative = signal), ``timed_out``, ``killed`` (needed SIGKILL),
    ``duration_s``, ``error`` when it could not start at all (``NOT_STARTED`` when the run was stopping), and the
    ``pump`` still copying its output when ``run_check`` was asked to leave it running (``finish_log``)."""

    status: int
    timed_out: bool
    killed: bool
    duration_s: float
    error: str | None = None
    pump: LogPump | None = field(default=None, compare=False, repr=False)

    @property
    def not_started(self) -> bool:
        """True when the runner refused to start the check (the run was stopping)."""
        return self.error == NOT_STARTED

    def finish_log(self) -> bool:
        """Finish the log pump (when one is running); True when the log was cut at the cap."""
        return self.pump.finish() if self.pump is not None else False


def expand(text: str, tokens: Mapping[str, str]) -> str:
    """Replace ``{root}``, ``{scratch}``, ``{tmp}``, ``{run}`` (the keys of ``tokens``) in one argv element or value."""
    out = text
    for key, value in tokens.items():
        out = out.replace("{" + key + "}", value)
    return out


def runner_marker() -> dict[str, str]:
    """``T111_RUNNER_PID`` and ``T111_RUNNER_START`` of this process."""
    me = hygiene.read_proc(os.getpid(), with_environ=False)
    return {"T111_RUNNER_PID": str(os.getpid()), "T111_RUNNER_START": str(me.start if me else 0)}


def check_env(check: Check, *, paths: Paths, run_id: str, tokens: Mapping[str, str],
              base: Mapping[str, str] | None = None, lock_dir: Path | None = None) -> dict[str, str]:
    """The environment of a check: the runner's, the check's ``env`` (tokens expanded), then the runner's markers."""
    env = dict(os.environ if base is None else base)
    env.pop("T111_HELPER", None)
    for key, value in check.env.items():
        env[key] = expand(value, tokens)
    env["TMPDIR"] = str(paths.tmp)
    env["T111_SCRATCH"] = str(paths.scratch)
    env["T111_RUN"] = run_id
    env["T111_CHECK"] = check.id
    env.update(runner_marker())
    env["T111_RESULTS"] = str(paths.results)
    if lock_dir is not None:
        env["T111_LOCKS"] = str(lock_dir)
    return env


def _signal_group(pgid: int, sig: int) -> None:
    try:
        os.killpg(pgid, sig)
    except (ProcessLookupError, PermissionError):
        pass


def run_check(check: Check, *, root: Path, paths: Paths, run_id: str, run_dir: Path,
              kill_grace_s: float = KILL_GRACE_S, on_start: Callable[[int], None] | None = None,
              start_lock: threading.Lock | threading.RLock | None = None, pass_fds: Sequence[int] = (),
              log_cap_mb: float = LOG_CAP_MB, drain_later: bool = False,
              cancelled: Callable[[], bool] | None = None, lock_dir: Path | None = None) -> Outcome:
    """Run ``check`` to completion (or its timeout) and return the outcome.

    ``on_start(pid)`` is called with the session leader's pid right after it starts, under ``start_lock`` when given
    and always under ``hygiene.SPAWNED.lock``, so the hygiene scan of a concurrent check never sees this process before
    it is registered as live and no reap can take its exit status. ``cancelled()`` is asked in that same critical
    section: True means the check is not started (``Outcome.error == NOT_STARTED``). ``pass_fds`` are inherited by the
    check (its lock fds). With ``drain_later`` the log pump is left running for the caller to ``finish_log`` (after it
    killed the check's leftovers, which may hold the pipe); otherwise it is finished here.
    """
    paths.tmp.mkdir(parents=True, exist_ok=True)
    tokens = {"root": str(root), "scratch": str(paths.scratch), "tmp": str(paths.tmp), "run": str(run_dir)}
    argv = [expand(part, tokens) for part in check.cmd]
    cwd = root / check.cwd
    env = check_env(check, paths=paths, run_id=run_id, tokens=tokens, lock_dir=lock_dir)
    began = time.monotonic()
    read_fd, write_fd = os.pipe()
    proc: subprocess.Popen | None = None
    try:
        if start_lock is not None:
            start_lock.acquire()
        try:
            with hygiene.SPAWNED.lock:
                if cancelled is not None and cancelled():
                    return Outcome(-signal.SIGKILL, False, False, time.monotonic() - began, error=NOT_STARTED)
                proc = hygiene.SPAWNED.popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=write_fd,
                                             stderr=write_fd, start_new_session=True, close_fds=True,
                                             pass_fds=tuple(pass_fds))
                if on_start is not None:
                    on_start(proc.pid)
        finally:
            if start_lock is not None:
                start_lock.release()
    except OSError as exc:
        with paths.log.open("ab") as log:
            log.write(f"t111: could not start {argv!r} in {cwd}: {exc}\n".encode())
        return Outcome(127, False, False, time.monotonic() - began, error=str(exc))
    finally:
        os.close(write_fd)
        if proc is None:
            os.close(read_fd)
    pump = LogPump(read_fd, paths.log, int(log_cap_mb * 1024 * 1024), log_cap_mb)
    timed_out = killed = False
    try:
        status = hygiene.SPAWNED.wait(proc, timeout=check.timeout_s)
    except subprocess.TimeoutExpired:
        timed_out = True
        _signal_group(proc.pid, signal.SIGTERM)
        try:
            status = hygiene.SPAWNED.wait(proc, timeout=kill_grace_s)
        except subprocess.TimeoutExpired:
            killed = True
            _signal_group(proc.pid, signal.SIGKILL)
            status = hygiene.SPAWNED.wait(proc)
        # whatever of the group outlived its leader gets the same treatment, now
        _signal_group(proc.pid, signal.SIGKILL)
    took = time.monotonic() - began
    if drain_later:
        return Outcome(status, timed_out, killed, took, pump=pump)
    pump.finish()
    return Outcome(status, timed_out, killed, took)
