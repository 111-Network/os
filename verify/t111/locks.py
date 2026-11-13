"""The box rules as locks (options.md §3.2): cargo slots and named exclusive locks, via ``fcntl.flock``.

Lock files live in ``${T111_LOCKS:-/tmp/111-verify-locks}``: the one place the runner writes outside ``--scratch``, on
purpose, so that two runner instances (two agents) share the box's limits. The directory is per box; when the runner
creates it, it gets mode 1777 (every user's runner shares it, like ``/tmp``).

- **Cargo slots**: a check with ``cargo = true`` holds one of ``cargo-slot-0.lock`` .. ``cargo-slot-<n-1>.lock``, where
  ``n`` is ``--cargo-slots`` (1 or 2, never more). Instances with different ``n`` still share slot 0 and 1, so the box
  never runs more than two cargo checks of ours at once. By design not counted: a cargo build started outside a runner.
- **Named locks**: ``<name>.lock`` held exclusively; every ``ns`` check takes ``verify`` (one verify harness at a time,
  which also keeps ``smoke.sh`` and ``panel-lint.sh`` apart).

Order: named locks in sorted order first, the cargo slot last; every holder follows it, so two checks never wait on each
other in a cycle. ``flock`` locks belong to an open file description, so two threads of one runner exclude each other
like two processes do.

The lock lives with the check (F2). The engine hands the held fds to the check (``pass_fds``), and a release only closes
the runner's fd (never ``LOCK_UN``, which would free the description for every holder): the lock stays held while any
process of the check keeps the fd, even when the runner itself died. So a wait is bounded and names its holder: after
taking a lock the holder writes ``run=<run id> pid=<runner pid> uid=<euid>`` into the lock file; a waiter reports one
line ``waiting for <lock> held by run <id> (pid <n>)`` and after ``wait_s`` (``--lock-wait``, default 1800 s) gives up
with ``LockTimeout``: ``lock <name> not free after N s (held by run <id>, pid <n>)``. The bound is for holders of other
or dead runs: a wait on this run's own holder (the holder line names this run) is not bounded (the engine's scheduler
never starts a check that needs a lock this run holds, so this is a leftover of one of its own checks).

A lock file another user's runner made may be opened read-only (``fs.protected_regular`` refuses ``O_CREAT`` on it):
the holder then cannot write its line into it and writes it into its own note file ``<name>.lock.holder.<euid>``. A
waiter names the newest mark (the lock file's line or a note file, by mtime): a note file is the words ``another
user's runner (holder not recorded in the lock file; its note: run <id>, pid <n>)``; no mark at all, with a lock file
we cannot write either, is ``another user's runner (holder not recorded)``.
"""

from __future__ import annotations

import fcntl
import os
import threading
import time
from contextlib import ExitStack, contextmanager
from pathlib import Path
from typing import Callable, Iterator

MAX_CARGO_SLOTS = 2
"""The box's limit of cargo builds at once (AGENTS.md)."""

DEFAULT_LOCK_DIR = "/tmp/111-verify-locks"
"""Where lock files live unless ``T111_LOCKS`` says otherwise."""

POLL_S = 0.05
"""How often a waiter retries a lock."""

LOCK_WAIT_S = 1800.0
"""How long a check waits for one lock before it FAILs (``--lock-wait``)."""


class LockTimeout(Exception):
    """A lock stayed taken for the whole wait: ``lock <name> not free after N s (held by run <id>, pid <n>)``."""

    def __init__(self, name: str, wait_s: float, holders: list[str]):
        self.name = name
        self.wait_s = wait_s
        self.holders = holders
        super().__init__(f"lock {name} not free after {wait_s:g} s (held by {'; '.join(holders)})")


class LockCancelled(Exception):
    """The run is stopping (an interrupt): the wait was abandoned."""


def lock_dir(env: dict[str, str] | None = None) -> Path:
    """The lock directory: ``$T111_LOCKS`` or ``/tmp/111-verify-locks``."""
    source = os.environ if env is None else env
    return Path(source.get("T111_LOCKS") or DEFAULT_LOCK_DIR)


def ensure_dir(directory: Path) -> None:
    """Create the lock directory (mode 1777 when this call creates it)."""
    if directory.is_dir():
        return
    directory.parent.mkdir(parents=True, exist_ok=True)
    try:
        directory.mkdir()
    except FileExistsError:
        return
    try:
        os.chmod(directory, 0o1777)
    except OSError:
        pass


def _open(path: Path) -> int:
    """Open (create) a lock file; read-only when another user's runner made it (``flock`` works on either; the
    holder line then goes into the holder's note file, ``_mark``)."""
    ensure_dir(path.parent)
    try:
        fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, 0o666)
    except PermissionError:
        return os.open(path, os.O_RDONLY | os.O_CLOEXEC)
    try:
        if os.fstat(fd).st_uid == os.geteuid():
            os.fchmod(fd, 0o666)
    except OSError:
        pass
    return fd


def _parse_mark(text: str) -> tuple[str, str] | None:
    fields = dict(part.split("=", 1) for part in text.split() if "=" in part)
    if "run" in fields and "pid" in fields:
        return fields["run"], fields["pid"]
    return None


def note_path(path: Path, uid: int | None = None) -> Path:
    """The note file a holder writes its line into when it cannot write the lock file: ``<lock>.holder.<euid>``."""
    return path.with_name(f"{path.name}.holder.{os.geteuid() if uid is None else uid}")


def newest_mark(path: Path) -> tuple[tuple[str, str] | None, bool]:
    """(run id, pid) of the newest holder mark of a lock (its line, or a note file, by mtime), and whether that mark is
    a note file (the holder could not write the lock file). (None, False) when there is no mark."""
    marks: list[tuple[float, tuple[str, str], bool]] = []
    candidates = [(path, False)] + [(p, True) for p in path.parent.glob(f"{path.name}.holder.*")]
    for candidate, is_note in candidates:
        try:
            stat = candidate.stat()
            text = candidate.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        mark = _parse_mark(text)
        if mark is not None:
            marks.append((stat.st_mtime_ns, mark, is_note))
    if not marks:
        return None, False
    _mtime, mark, is_note = max(marks, key=lambda item: item[0])
    return mark, is_note


def read_holder(path: Path) -> tuple[str, str] | None:
    """(run id, pid) of the newest holder mark of a lock (``newest_mark``), or None."""
    return newest_mark(path)[0]


def _holder_words(path: Path, waiting: bool) -> str:
    holder, is_note = newest_mark(path)
    if holder is None:
        if not os.access(path, os.W_OK):
            return "another user's runner (holder not recorded)"
        return "an unknown holder"
    run_id, pid = holder
    if is_note:
        return f"another user's runner (holder not recorded in the lock file; its note: run {run_id}, pid {pid})"
    return f"run {run_id} (pid {pid})" if waiting else f"run {run_id}, pid {pid}"


class Held(list):
    """The names of the locks a check holds (a list, as before) plus ``fds``: the open lock fds to hand to the check."""

    fds: list[int]

    def __init__(self) -> None:
        super().__init__()
        self.fds = []


class Locks:
    """The locks of one runner instance: ``slots`` cargo slots (1 or 2) in ``directory``.

    ``run_id`` and ``pid`` are written into a lock file once it is taken (the holder line a waiter reports); ``wait_s``
    bounds every wait.
    """

    def __init__(self, directory: Path, slots: int = 1, *, run_id: str = "", pid: int | None = None,
                 wait_s: float = LOCK_WAIT_S):
        if not 1 <= slots <= MAX_CARGO_SLOTS:
            raise ValueError(f"cargo slots must be 1 or 2, got {slots}")
        self.directory = directory
        self.slots = slots
        self.run_id = run_id or "-"
        self.pid = os.getpid() if pid is None else pid
        self.wait_s = wait_s

    def _named(self, name: str) -> Path:
        return self.directory / f"{name}.lock"

    def _slot(self, index: int) -> Path:
        return self.directory / f"cargo-slot-{index}.lock"

    def _mark(self, fd: int, path: Path) -> None:
        """Write the holder line into the lock file, or (a read-only fd) into this holder's note file."""
        line = f"run={self.run_id} pid={self.pid} uid={os.geteuid()}\n".encode()
        try:
            os.ftruncate(fd, 0)
            os.pwrite(fd, line, 0)
            return
        except OSError:
            pass
        note = note_path(path)
        try:
            tmp = note.with_name(note.name + f".{os.getpid()}")
            tmp.write_bytes(line)
            os.replace(tmp, note)
        except OSError:
            pass

    def _ours(self, paths: list[Path]) -> bool:
        """True when every one of ``paths`` is held by this run (its newest mark names this run id)."""
        for path in paths:
            holder, _note = newest_mark(path)
            if holder is None or holder[0] != self.run_id or self.run_id == "-":
                return False
        return True

    def _acquire(self, paths: list[Path], fds: list[int], cancel: threading.Event | None,
                 on_wait: Callable[[str], None] | None) -> int:
        """Take the first free of ``fds`` (polling); return its index. Raises ``LockTimeout`` / ``LockCancelled``."""
        deadline = time.monotonic() + self.wait_s
        announced = False
        while True:
            for index, fd in enumerate(fds):
                try:
                    fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                except BlockingIOError:
                    continue
                self._mark(fd, paths[index])
                return index
            if cancel is not None and cancel.is_set():
                raise LockCancelled(", ".join(p.stem for p in paths))
            if not announced:
                announced = True
                if on_wait is not None:
                    held = "; ".join(f"{p.stem} held by {_holder_words(p, True)}" for p in paths)
                    on_wait(f"waiting for {held}")
            if time.monotonic() >= deadline:
                if self._ours(paths):  # this run's own holder: the wait is not bounded
                    deadline = time.monotonic() + self.wait_s
                else:
                    raise LockTimeout("|".join(p.stem for p in paths), self.wait_s,
                                      [_holder_words(p, False) for p in paths])
            time.sleep(POLL_S)

    def take_named(self, name: str, *, cancel: threading.Event | None = None,
                   on_wait: Callable[[str], None] | None = None) -> int:
        """Take the exclusive lock ``name``; return its fd (``release`` closes it)."""
        path = self._named(name)
        fd = _open(path)
        try:
            self._acquire([path], [fd], cancel, on_wait)
        except BaseException:
            os.close(fd)
            raise
        return fd

    def take_slot(self, *, cancel: threading.Event | None = None,
                  on_wait: Callable[[str], None] | None = None) -> tuple[int, int]:
        """Take one free cargo slot; return (index, fd)."""
        paths = [self._slot(i) for i in range(self.slots)]
        fds = [_open(p) for p in paths]
        try:
            index = self._acquire(paths, fds, cancel, on_wait)
        except BaseException:
            for fd in fds:
                os.close(fd)
            raise
        for i, fd in enumerate(fds):
            if i != index:
                os.close(fd)
        return index, fds[index]

    @staticmethod
    def release(fd: int) -> None:
        """Close the runner's fd of a held lock (no ``LOCK_UN``: a process of the check that kept the fd keeps it)."""
        try:
            os.close(fd)
        except OSError:
            pass

    @contextmanager
    def named(self, name: str) -> Iterator[None]:
        """Hold the exclusive lock ``name`` (waits up to ``wait_s``)."""
        fd = self.take_named(name)
        try:
            yield
        finally:
            self.release(fd)

    @contextmanager
    def cargo_slot(self) -> Iterator[int]:
        """Hold one free cargo slot; yields its index (waits up to ``wait_s``)."""
        index, fd = self.take_slot()
        try:
            yield index
        finally:
            self.release(fd)

    @contextmanager
    def for_check(self, names: tuple[str, ...], cargo: bool, *, cancel: threading.Event | None = None,
                  on_wait: Callable[[str], None] | None = None) -> Iterator[Held]:
        """Take a check's named locks (sorted) then, when ``cargo``, a cargo slot; yields the names held (``Held``,
        whose ``fds`` the engine hands to the check)."""
        held = Held()
        with ExitStack() as stack:
            for name in sorted(set(names)):
                fd = self.take_named(name, cancel=cancel, on_wait=on_wait)
                stack.callback(self.release, fd)
                held.append(name)
                held.fds.append(fd)
            if cargo:
                index, fd = self.take_slot(cancel=cancel, on_wait=on_wait)
                stack.callback(self.release, fd)
                held.append(f"cargo-slot-{index}")
                held.fds.append(fd)
            yield held
