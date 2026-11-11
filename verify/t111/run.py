"""One check = one subprocess session (options.md §3.2).

``run_check`` starts the check's argv in its own session (``start_new_session``), from its cwd under the repo root,
with stdout and stderr on one file descriptor (``out.log``: one combined, ordered log), stdin from ``/dev/null``, and
the environment the check declares on top of the runner's, plus:

- ``TMPDIR=<scratch>/tmp`` (every harness ``mktemp`` lands inside the check's scratch),
- ``T111_SCRATCH=<scratch>``, ``T111_RUN=<run-id>``, ``T111_CHECK=<check id>`` (the hygiene markers),
- ``T111_RESULTS=<scratch>/emit.jsonl`` (the shared emitter's output file, F3 phase 2).

At ``timeout_s`` the whole process group gets SIGTERM, then SIGKILL after ``kill_grace_s`` (10 s). The outcome carries
the exit status, whether it timed out, and the duration (for the record only: no pass/fail decision reads it).
"""

from __future__ import annotations

import os
import signal
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Mapping

from .schema import Check

KILL_GRACE_S = 10.0
"""Seconds between SIGTERM and SIGKILL when a check times out."""


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


@dataclass(frozen=True)
class Outcome:
    """What happened to the process: ``status`` (negative = signal), ``timed_out``, ``killed`` (needed SIGKILL),
    ``duration_s``, and ``error`` when it could not start at all."""

    status: int
    timed_out: bool
    killed: bool
    duration_s: float
    error: str | None = None


def expand(text: str, tokens: Mapping[str, str]) -> str:
    """Replace ``{root}``, ``{scratch}``, ``{tmp}``, ``{run}`` (the keys of ``tokens``) in one argv element or value."""
    out = text
    for key, value in tokens.items():
        out = out.replace("{" + key + "}", value)
    return out


def check_env(check: Check, *, paths: Paths, run_id: str, tokens: Mapping[str, str],
              base: Mapping[str, str] | None = None) -> dict[str, str]:
    """The environment of a check: the runner's, the check's ``env`` (tokens expanded), then the runner's markers."""
    env = dict(os.environ if base is None else base)
    env.pop("T111_HELPER", None)
    for key, value in check.env.items():
        env[key] = expand(value, tokens)
    env["TMPDIR"] = str(paths.tmp)
    env["T111_SCRATCH"] = str(paths.scratch)
    env["T111_RUN"] = run_id
    env["T111_CHECK"] = check.id
    env["T111_RESULTS"] = str(paths.results)
    return env


def _signal_group(pgid: int, sig: int) -> None:
    try:
        os.killpg(pgid, sig)
    except (ProcessLookupError, PermissionError):
        pass


def run_check(check: Check, *, root: Path, paths: Paths, run_id: str, run_dir: Path,
              kill_grace_s: float = KILL_GRACE_S, on_start: Callable[[int], None] | None = None,
              start_lock: threading.Lock | None = None) -> Outcome:
    """Run ``check`` to completion (or its timeout) and return the outcome.

    ``on_start(pid)`` is called with the session leader's pid right after it starts, under ``start_lock`` when given, so
    the hygiene scan of a concurrent check never sees this process before it is registered as live.
    """
    paths.tmp.mkdir(parents=True, exist_ok=True)
    tokens = {"root": str(root), "scratch": str(paths.scratch), "tmp": str(paths.tmp), "run": str(run_dir)}
    argv = [expand(part, tokens) for part in check.cmd]
    cwd = root / check.cwd
    env = check_env(check, paths=paths, run_id=run_id, tokens=tokens)
    began = time.monotonic()
    with paths.log.open("ab") as log:
        try:
            if start_lock is not None:
                start_lock.acquire()
            try:
                proc = subprocess.Popen(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, stdout=log,
                                        stderr=subprocess.STDOUT, start_new_session=True, close_fds=True)
                if on_start is not None:
                    on_start(proc.pid)
            finally:
                if start_lock is not None:
                    start_lock.release()
        except OSError as exc:
            log.write(f"t111: could not start {argv!r} in {cwd}: {exc}\n".encode())
            return Outcome(127, False, False, time.monotonic() - began, error=str(exc))
        timed_out = killed = False
        try:
            status = proc.wait(timeout=check.timeout_s)
        except subprocess.TimeoutExpired:
            timed_out = True
            _signal_group(proc.pid, signal.SIGTERM)
            try:
                status = proc.wait(timeout=kill_grace_s)
            except subprocess.TimeoutExpired:
                killed = True
                _signal_group(proc.pid, signal.SIGKILL)
                status = proc.wait()
            if timed_out:
                # whatever of the group outlived its leader gets the same treatment, now
                _signal_group(proc.pid, signal.SIGKILL)
    return Outcome(status, timed_out, killed, time.monotonic() - began)
