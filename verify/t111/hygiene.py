"""Hygiene around a check (options.md §3.2): leftovers, listeners, the tree, ports, and the capability probes.

- **Leftovers.** The runner makes itself a child subreaper (``prctl(PR_SET_CHILD_SUBREAPER)``), so a process a check
  left behind (backgrounded, double-forked, ``setsid``) stays the runner's descendant instead of moving to init. After a
  check ends, ``find_leftovers`` reads ``/proc`` and returns every process that (1) descends from the runner and belongs
  to no check still running, (2) carries this run's marker (``T111_RUN``/``T111_CHECK``) in its environ, or (3) names
  the check's scratch dir in its cmdline or environ. Those become a ``hygiene/<check>`` FAIL naming pid and cmdline and
  are killed. A process whose binary is ours (``111``, ``111-agent``, ``stub-*.py``, rnsd, meshmud, a headless
  browser, vite) that appeared during the check but carries none of the above is reported, not killed: nothing proves
  it is not another agent's.
- **Listeners.** The listening TCP sockets of ``/proc/net/tcp`` and ``tcp6``, joined to a pid through
  ``/proc/<pid>/fd``; a leftover's listening ports are named in its FAIL.
- **Ports.** ``port_holders`` is the pre-check of an ``ns`` check (11180, 11183, 11230, 11280 by default).
- **Tree.** ``tree_state`` is ``git status --porcelain`` plus a content hash per listed path; ``tree_diff`` names the
  paths a check changed.
- **Probes.** ``Probes`` answers a check's ``needs`` (``userns``, ``netns``, ``root``, ``nonroot``, ``network``, any
  tool on PATH), once per run.

Every helper subprocess (git, unshare) runs with ``T111_HELPER=<run-id>`` in its environment so the leftover scan never
mistakes it for a check's process.
"""

from __future__ import annotations

import ctypes
import fnmatch
import hashlib
import os
import shutil
import signal
import subprocess
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Mapping

NS_PORTS: tuple[int, ...] = (11180, 11183, 11230, 11280)
"""The fixed ports of the namespace harnesses; an ``ns`` check is SKIPPED when one is held."""

OUR_BINARIES: frozenset[str] = frozenset({"111", "111d", "111-agent", "rnsd", "meshmud", "llama-server"})
"""Executable basenames that are ours."""

OUR_SCRIPT_GLOBS: tuple[str, ...] = ("stub-*.py", "stub_*.py", "fake-*.py", "fake_*.py")
"""Script basenames (any argv element) that are ours."""

PR_SET_CHILD_SUBREAPER = 36


@dataclass
class Proc:
    """One process as read from ``/proc``. ``environ`` is None when it could not be read."""

    pid: int
    ppid: int
    pgid: int
    sid: int
    state: str
    start: int
    argv: list[str]
    environ: dict[str, str] | None = field(default=None, repr=False)

    @property
    def key(self) -> tuple[int, int]:
        """(pid, start time in ticks): survives pid reuse."""
        return (self.pid, self.start)

    @property
    def cmdline(self) -> str:
        """The argv joined by spaces (``[<name>]`` for a kernel thread or a zombie)."""
        return " ".join(self.argv) if self.argv else "[?]"


@dataclass
class Leftover:
    """A process a check left behind: the check it is attributed to, why, its ports, and whether it gets killed."""

    proc: Proc
    owner: str
    rule: str
    ports: list[int]
    kill: bool

    def describe(self) -> str:
        """``pid 123 (sleep 300)`` plus ``listening :11180`` when it listens."""
        text = f"pid {self.proc.pid} ({self.proc.cmdline[:200]})"
        if self.ports:
            text += " listening " + ",".join(f":{p}" for p in self.ports)
        if not self.kill:
            text += " [not killed: our binary, but nothing ties it to this run]"
        return text


def set_subreaper() -> bool:
    """Make this process a child subreaper; False when the kernel or libc refuses."""
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        return libc.prctl(PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) == 0
    except (OSError, AttributeError):
        return False


def _read(path: str) -> bytes | None:
    try:
        with open(path, "rb") as fh:
            return fh.read()
    except OSError:
        return None


def read_proc(pid: int, *, with_environ: bool = True) -> Proc | None:
    """Read one process, or None when it is gone."""
    stat = _read(f"/proc/{pid}/stat")
    if stat is None:
        return None
    text = stat.decode("utf-8", "replace")
    close = text.rfind(")")
    if close < 0:
        return None
    fields = text[close + 2:].split()
    try:
        state, ppid, pgid, sid, start = fields[0], int(fields[1]), int(fields[2]), int(fields[3]), int(fields[19])
    except (IndexError, ValueError):
        return None
    raw_cmd = _read(f"/proc/{pid}/cmdline") or b""
    argv = [a.decode("utf-8", "replace") for a in raw_cmd.split(b"\0") if a]
    environ: dict[str, str] | None = None
    if with_environ:
        raw_env = _read(f"/proc/{pid}/environ")
        if raw_env is not None:
            environ = {}
            for item in raw_env.split(b"\0"):
                if b"=" in item:
                    key, _, value = item.partition(b"=")
                    environ[key.decode("utf-8", "replace")] = value.decode("utf-8", "replace")
    return Proc(pid, ppid, pgid, sid, state, start, argv, environ)


def snapshot(*, with_environ: bool = True) -> dict[int, Proc]:
    """Every process visible in ``/proc``."""
    out: dict[int, Proc] = {}
    for entry in os.listdir("/proc"):
        if entry.isdigit():
            proc = read_proc(int(entry), with_environ=with_environ)
            if proc is not None:
                out[proc.pid] = proc
    return out


def process_keys() -> set[tuple[int, int]]:
    """(pid, start) of every process: the cheap snapshot taken before a check."""
    return {p.key for p in snapshot(with_environ=False).values()}


def ancestors(procs: Mapping[int, Proc], pid: int) -> list[int]:
    """The parent chain of ``pid`` (nearest first), as far as ``procs`` knows it."""
    chain: list[int] = []
    seen = {pid}
    cur = procs.get(pid)
    while cur is not None and cur.ppid > 0 and cur.ppid not in seen:
        chain.append(cur.ppid)
        seen.add(cur.ppid)
        cur = procs.get(cur.ppid)
    return chain


def is_ours(proc: Proc) -> bool:
    """True when the process's binary or script is one of ours."""
    if not proc.argv:
        return False
    names = [os.path.basename(a) for a in proc.argv[:3]]
    if names[0] in OUR_BINARIES:
        return True
    for name in names:
        if any(fnmatch.fnmatchcase(name, g) for g in OUR_SCRIPT_GLOBS):
            return True
    if ("chrome" in names[0] or "chromium" in names[0]) and any(a.startswith("--headless") for a in proc.argv):
        return True
    return any(n in ("vite", "vite.js") for n in names)


def listening_inodes() -> dict[int, int]:
    """Socket inode -> port of every listening TCP socket (IPv4 and IPv6) in this network namespace."""
    out: dict[int, int] = {}
    for table in ("/proc/net/tcp", "/proc/net/tcp6"):
        raw = _read(table)
        if raw is None:
            continue
        for line in raw.decode("ascii", "replace").splitlines()[1:]:
            parts = line.split()
            if len(parts) < 10 or parts[3] != "0A":
                continue
            try:
                port = int(parts[1].rsplit(":", 1)[1], 16)
                inode = int(parts[9])
            except (IndexError, ValueError):
                continue
            out[inode] = port
    return out


def socket_inodes(pid: int) -> set[int]:
    """The socket inodes a process holds open."""
    out: set[int] = set()
    try:
        fds = os.listdir(f"/proc/{pid}/fd")
    except OSError:
        return out
    for fd in fds:
        try:
            target = os.readlink(f"/proc/{pid}/fd/{fd}")
        except OSError:
            continue
        if target.startswith("socket:["):
            try:
                out.add(int(target[8:-1]))
            except ValueError:
                pass
    return out


def ports_of(pid: int, listening: Mapping[int, int] | None = None) -> list[int]:
    """The ports a process listens on."""
    table = listening_inodes() if listening is None else listening
    return sorted({table[i] for i in socket_inodes(pid) if i in table})


def port_holders(ports: Iterable[int]) -> dict[int, tuple[int | None, str]]:
    """For each held port of ``ports``: (holder pid or None, holder cmdline or ``unknown``)."""
    wanted = set(ports)
    listening = {inode: port for inode, port in listening_inodes().items() if port in wanted}
    if not listening:
        return {}
    held: dict[int, tuple[int | None, str]] = {port: (None, "unknown") for port in listening.values()}
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        pid = int(entry)
        for inode in socket_inodes(pid):
            port = listening.get(inode)
            if port is not None and held[port][0] is None:
                proc = read_proc(pid, with_environ=False)
                held[port] = (pid, proc.cmdline if proc else "unknown")
    return held


def find_leftovers(*, runner_pid: int, run_id: str, finished: str, scratch: Path, live: Mapping[str, int],
                   before: set[tuple[int, int]]) -> list[Leftover]:
    """The processes check ``finished`` left behind (see the module docstring for the three rules and the report-only
    fourth). ``live`` maps each check still running to its session leader pid; ``before`` is the process snapshot taken
    when ``finished`` started."""
    procs = snapshot()
    listening = listening_inodes()
    scratch_s = str(scratch)
    live_roots = set(live.values())
    runner = procs.get(runner_pid)
    out: list[Leftover] = []
    for proc in procs.values():
        if proc.pid == runner_pid or proc.state == "Z":
            continue
        if runner is not None and proc.ppid == runner_pid and proc.argv == runner.argv:
            continue  # the runner forking a helper or a check, caught between fork and exec
        env = proc.environ or {}
        if env.get("T111_HELPER"):
            continue
        chain = ancestors(procs, proc.pid)
        if proc.pid in live_roots or proc.sid in live_roots or live_roots.intersection(chain):
            continue
        same_run = env.get("T111_RUN") == run_id
        marker = env.get("T111_CHECK") if same_run else None
        if marker is not None and marker in live:
            continue
        names_scratch = scratch_s in proc.cmdline or any(scratch_s in v for v in env.values())
        owner = marker or finished
        rule = ""
        if runner_pid in chain:
            rule = "descends from the runner"
        elif same_run:
            rule = "carries this run's marker"
        elif names_scratch:
            rule = "names the check's scratch dir"
        if rule:
            out.append(Leftover(proc, owner, rule, sorted({listening[i] for i in socket_inodes(proc.pid)
                                                           if i in listening}), True))
            continue
        if is_ours(proc) and proc.key not in before and not env.get("T111_RUN"):
            out.append(Leftover(proc, finished, "our binary, appeared during the check",
                                sorted({listening[i] for i in socket_inodes(proc.pid) if i in listening}), False))
    return sorted(out, key=lambda lo: lo.proc.pid)


def kill_leftovers(leftovers: Iterable[Leftover], runner_pid: int, wait_s: float = 2.0) -> None:
    """SIGKILL every leftover marked ``kill`` and reap the ones that are the runner's children."""
    victims = [lo.proc for lo in leftovers if lo.kill]
    for proc in victims:
        try:
            os.kill(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        except PermissionError:
            pass
    deadline = time.monotonic() + wait_s
    pending = {p.pid for p in victims}
    while pending and time.monotonic() < deadline:
        for pid in list(pending):
            now = read_proc(pid, with_environ=False)
            if now is None:
                pending.discard(pid)
            elif now.state == "Z":
                if now.ppid == runner_pid:
                    reap(pid)
                pending.discard(pid)
        if pending:
            time.sleep(0.02)
    reap_orphans(runner_pid)


def reap(pid: int) -> None:
    """Collect one dead child's status (only ever called for a pid that is not a ``subprocess`` child)."""
    try:
        os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        pass


def reap_orphans(runner_pid: int, keep: Iterable[int] = ()) -> None:
    """Reap zombies the subreaper inherited (never a pid in ``keep``: those ``subprocess`` still waits for)."""
    skip = set(keep)
    for proc in snapshot(with_environ=False).values():
        if proc.state == "Z" and proc.ppid == runner_pid and proc.pid not in skip:
            reap(proc.pid)


# ---------------------------------------------------------------------------------------------------------------------
# the tree


def helper_run(argv: list[str], *, cwd: Path, run_id: str, timeout: float = 120.0) -> subprocess.CompletedProcess:
    """Run one helper command of the runner itself (marked, own session, explicit cwd and timeout)."""
    env = dict(os.environ)
    env["T111_HELPER"] = run_id or "1"
    env.pop("T111_RUN", None)
    env.pop("T111_CHECK", None)
    return subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL, capture_output=True, timeout=timeout,
                          start_new_session=True, check=False)


def _hash(path: Path) -> str:
    try:
        if path.is_symlink():
            return "link:" + os.readlink(path)
        if path.is_dir():
            return "dir"
        digest = hashlib.sha256()
        with path.open("rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 16), b""):
                digest.update(chunk)
        return digest.hexdigest()
    except FileNotFoundError:
        return "missing"
    except OSError as exc:
        return f"error:{exc.errno}"


def tree_state(root: Path, run_id: str = "") -> dict[str, str] | None:
    """``{path: "<XY>:<content hash>"}`` for every path ``git status`` lists, or None when ``root`` is no work tree."""
    if shutil.which("git") is None:
        return None
    try:
        done = helper_run(["git", "--no-optional-locks", "-C", str(root), "status", "--porcelain=v1", "-z",
                           "--untracked-files=all"], cwd=root, run_id=run_id)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if done.returncode != 0:
        return None
    entries = done.stdout.decode("utf-8", "surrogateescape").split("\0")
    state: dict[str, str] = {}
    i = 0
    while i < len(entries):
        item = entries[i]
        i += 1
        if len(item) < 4:
            continue
        xy, path = item[:2], item[3:]
        if xy[0] in "RC":
            i += 1  # the next entry is the source path of the rename or copy
        state[path] = f"{xy}:{_hash(root / path)}"
    return state


def tree_diff(before: Mapping[str, str] | None, after: Mapping[str, str] | None) -> list[str]:
    """The paths whose ``git status`` line or content changed between two ``tree_state`` snapshots."""
    if before is None or after is None:
        return []
    return sorted(p for p in set(before) | set(after) if before.get(p) != after.get(p))


# ---------------------------------------------------------------------------------------------------------------------
# capability probes


class Probes:
    """Answers ``needs`` once per run: ``userns``/``netns``, ``root``, ``nonroot``, ``network``, or a tool on PATH."""

    NAMED: tuple[str, ...] = ("userns", "netns", "root", "nonroot", "network")

    def __init__(self, *, offline: bool, run_id: str, cwd: Path):
        self.offline = offline
        self.run_id = run_id
        self.cwd = cwd
        self._lock = threading.Lock()
        self._cache: dict[str, bool] = {}

    def _probe(self, need: str) -> bool:
        if need in ("userns", "netns"):
            if shutil.which("unshare") is None:
                return False
            try:
                done = helper_run(["unshare", "-Urnm", "true"], cwd=self.cwd, run_id=self.run_id, timeout=20)
            except (OSError, subprocess.TimeoutExpired):
                return False
            return done.returncode == 0
        if need == "root":
            return os.geteuid() == 0
        if need == "nonroot":
            return os.geteuid() != 0
        if need == "network":
            return not self.offline
        return shutil.which(need) is not None

    def has(self, need: str) -> bool:
        """True when the need is met (cached)."""
        with self._lock:
            if need not in self._cache:
                self._cache[need] = self._probe(need)
            return self._cache[need]

    def missing(self, needs: Iterable[str]) -> list[str]:
        """The needs that are not met, in order."""
        return [n for n in needs if not self.has(n)]

    def found(self) -> dict[str, bool]:
        """Every need probed so far."""
        with self._lock:
            return dict(self._cache)
