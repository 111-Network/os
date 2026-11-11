"""The box rules as locks (options.md §3.2): cargo slots and named exclusive locks, via ``fcntl.flock``.

Lock files live in ``${T111_LOCKS:-/tmp/111-verify-locks}``: the one place the runner writes outside ``--scratch``, on
purpose, so that two runner instances (two agents) share the box's limits.

- **Cargo slots**: a check with ``cargo = true`` holds one of ``cargo-slot-0.lock`` .. ``cargo-slot-<n-1>.lock``, where
  ``n`` is ``--cargo-slots`` (1 or 2, never more). Instances with different ``n`` still share slot 0 and 1, so the box
  never runs more than two cargo checks of ours at once.
- **Named locks**: ``<name>.lock`` held exclusively; every ``ns`` check takes ``verify`` (one verify harness at a time,
  which also keeps ``smoke.sh`` and ``panel-lint.sh`` apart).

Order: named locks in sorted order first, the cargo slot last; every holder follows it, so two checks never wait on each
other in a cycle. ``flock`` locks belong to an open file description, so two threads of one runner exclude each other
like two processes do.
"""

from __future__ import annotations

import fcntl
import os
import time
from contextlib import ExitStack, contextmanager
from pathlib import Path
from typing import Iterator

MAX_CARGO_SLOTS = 2
"""The box's limit of cargo builds at once (AGENTS.md)."""

DEFAULT_LOCK_DIR = "/tmp/111-verify-locks"
"""Where lock files live unless ``T111_LOCKS`` says otherwise."""

POLL_S = 0.05
"""How often a waiter retries the cargo slots; the wait itself is unbounded (a holder's timeout ends it)."""


def lock_dir(env: dict[str, str] | None = None) -> Path:
    """The lock directory: ``$T111_LOCKS`` or ``/tmp/111-verify-locks``."""
    source = os.environ if env is None else env
    return Path(source.get("T111_LOCKS") or DEFAULT_LOCK_DIR)


def _open(path: Path) -> int:
    path.parent.mkdir(parents=True, exist_ok=True)
    return os.open(path, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC, 0o666)


class Locks:
    """The locks of one runner instance: ``slots`` cargo slots (1 or 2) in ``directory``."""

    def __init__(self, directory: Path, slots: int = 1):
        if not 1 <= slots <= MAX_CARGO_SLOTS:
            raise ValueError(f"cargo slots must be 1 or 2, got {slots}")
        self.directory = directory
        self.slots = slots

    def _named(self, name: str) -> Path:
        return self.directory / f"{name}.lock"

    def _slot(self, index: int) -> Path:
        return self.directory / f"cargo-slot-{index}.lock"

    @contextmanager
    def named(self, name: str) -> Iterator[None]:
        """Hold the exclusive lock ``name`` (blocks until free)."""
        fd = _open(self._named(name))
        try:
            fcntl.flock(fd, fcntl.LOCK_EX)
            yield
        finally:
            fcntl.flock(fd, fcntl.LOCK_UN)
            os.close(fd)

    @contextmanager
    def cargo_slot(self) -> Iterator[int]:
        """Hold one free cargo slot; yields its index (blocks, polling, until one is free)."""
        fds = [_open(self._slot(i)) for i in range(self.slots)]
        held = -1
        try:
            while held < 0:
                for index, fd in enumerate(fds):
                    try:
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    except BlockingIOError:
                        continue
                    held = index
                    break
                if held < 0:
                    time.sleep(POLL_S)
            yield held
        finally:
            if held >= 0:
                fcntl.flock(fds[held], fcntl.LOCK_UN)
            for fd in fds:
                os.close(fd)

    @contextmanager
    def for_check(self, names: tuple[str, ...], cargo: bool) -> Iterator[list[str]]:
        """Take a check's named locks (sorted) then, when ``cargo``, a cargo slot; yields the names held."""
        held: list[str] = []
        with ExitStack() as stack:
            for name in sorted(set(names)):
                stack.enter_context(self.named(name))
                held.append(name)
            if cargo:
                index = stack.enter_context(self.cargo_slot())
                held.append(f"cargo-slot-{index}")
            yield held
