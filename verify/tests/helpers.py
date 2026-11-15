"""Shared fixtures of the runner's tests: a temp git repo with the fake check scripts, and a runner subprocess.

``Sandbox`` lays out ``<tmp>/tree`` (a git repo holding ``fx/*.sh``, committed, so it starts clean), ``<tmp>/scratch``
(the ``--scratch`` dir, with the lock dir ``scratch/locks`` inside it) and ``<tmp>/events.log`` (the lock-free event
file the ``stamp`` checks append to). ``run()`` starts ``verify/run.py`` as its own process with ``--root`` and
``--map`` pointing at the sandbox and the fixture map.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

VERIFY = Path(__file__).resolve().parent.parent
RUN_PY = VERIFY / "run.py"
FIXTURE_MAP = VERIFY / "tests" / "fixtures" / "map"

STAMP_PY = r'''
import os, sys, time
fd = os.open(sys.argv[1], os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o644)
os.write(fd, f"{sys.argv[2]} {sys.argv[3]} {time.monotonic_ns()}\n".encode())
os.close(fd)
'''

FX_SCRIPTS: dict[str, str] = {
    "pass.sh": "echo 'ok: the fixture passes'\n",
    "fail.sh": "echo 'ok: first line'\necho 'FAIL: the fixture says no  [HTTP 502: upstream]' >&2\nexit 3\n",
    "hang.sh": "echo 'started, now hanging'\nsleep 30\necho 'never printed'\n",
    "leak.sh": "sleep 300 &\nsetsid sleep 301 &\nsleep 0.5\necho 'left two sleepers behind'\n",
    "dirty.sh": "echo dirt > fx-dirty.txt\necho 'wrote into the tree'\n",
    ".gitignore": "ignored-dir/\n",
    "ignored.sh": "mkdir -p fx/ignored-dir\necho cache > fx/ignored-dir/new.txt\necho 'wrote an ignored file into the tree'\n",
    "writer.sh": "mktemp >/dev/null\necho data > \"$T111_SCRATCH/own.txt\"\necho data > \"$TMPDIR/t.txt\"\n",
    "stamp.sh": ("python3 -I fx/stamp.py \"$EVENTS\" start \"$T111_CHECK\"\nsleep 0.4\n"
                 "python3 -I fx/stamp.py \"$EVENTS\" end \"$T111_CHECK\"\n"),
    "stamp.py": STAMP_PY,
    # X2b fixtures
    "quickfail.sh": "echo 'error: quick failure' >&2\nsleep 0.2\nexit 3\n",
    "long.sh": ("setsid sleep 302 >/dev/null 2>&1 &\nsleep 303 >/dev/null 2>&1 &\necho 'long: started'\n"
                "sleep 60\necho 'never printed'\n"),
    "gate.sh": ("echo 'long: waiting for the go file'\ni=0\nwhile [ ! -e \"$T111_GO\" ] && [ $i -lt 600 ]; do\n"
                "  sleep 0.05\n  i=$((i + 1))\ndone\necho 'long: go'\n"),
    "big.sh": "head -c 3000000 /dev/zero | tr '\\0' 'x'\necho\necho 'big: the end'\n",
    "badname.sh": "printf 'x' > \"$(printf 'bad-\\377.txt')\"\necho 'wrote a file whose name is not UTF-8'\n",
}


def max_overlap(events: Path, ids: set[str] | None = None) -> int:
    """The largest number of stamped checks between their start and end at any moment."""
    rows = []
    for line in events.read_text().splitlines():
        what, check, ns = line.split()
        if ids is None or check in ids:
            rows.append((int(ns), 0 if what == "end" else 1, check))
    rows.sort()
    running = best = 0
    for _ns, start, _check in rows:
        running += 1 if start else -1
        best = max(best, running)
    return best


class Sandbox:
    """A temp dir with a git tree of fake checks, a scratch dir and a lock dir."""

    def __init__(self, test: unittest.TestCase):
        self.extra_env: dict[str, str] = {}
        self.base = Path(tempfile.mkdtemp(prefix="t111-test-"))
        test.addCleanup(shutil.rmtree, self.base, True)
        self.tree = self.base / "tree"
        self.scratch = self.base / "scratch"
        self.locks = self.scratch / "locks"
        self.events = self.base / "events.log"
        self.map_dir = FIXTURE_MAP
        (self.tree / "fx").mkdir(parents=True)
        for name, body in FX_SCRIPTS.items():
            (self.tree / "fx" / name).write_text(body)
        git = ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"]
        for argv in (["git", "init", "-q", "."], ["git", "add", "-A"], git + ["commit", "-q", "-m", "fx"]):
            subprocess.run(argv, cwd=self.tree, check=True, timeout=60, capture_output=True, start_new_session=True)

    def env(self) -> dict[str, str]:
        env = dict(os.environ)
        env["T111_LOCKS"] = str(self.locks)
        env["EVENTS"] = str(self.events)
        for key in ("T111_RUN", "T111_CHECK", "T111_SCRATCH", "T111_HELPER", "T111_RESULTS", "T111_RUNNER_PID",
                    "T111_RUNNER_START", "T111_SHARED_BOX"):
            env.pop(key, None)
        env.update(self.extra_env)
        return env

    def argv(self, *args: str, scratch: bool = True) -> list[str]:
        out = [sys.executable, "-I", str(RUN_PY), "--root", str(self.tree), "--map", str(self.map_dir)]
        if scratch:
            out += ["--scratch", str(self.scratch)]
        return out + list(args)

    def run(self, *args: str, scratch: bool = True, timeout: float = 60) -> subprocess.CompletedProcess:
        """Run ``verify/run.py`` with ``args`` against this sandbox; text output captured."""
        return subprocess.run(self.argv(*args, scratch=scratch), cwd=self.base, env=self.env(), text=True,
                              capture_output=True, timeout=timeout, start_new_session=True, check=False)

    def run_dirs(self) -> list[Path]:
        if not self.scratch.is_dir():
            return []
        return sorted(d for d in self.scratch.iterdir() if d.is_dir() and d.name != "locks")

    def results(self) -> list[dict]:
        """The records of the only (or latest) run."""
        import json
        dirs = self.run_dirs()
        assert dirs, "no run dir"
        with (dirs[-1] / "results.jsonl").open() as fh:
            return [json.loads(line) for line in fh if line.strip()]

    def copy_map(self) -> Path:
        """A writable copy of the fixture map (for a test that edits it)."""
        dest = self.base / "map"
        shutil.copytree(FIXTURE_MAP, dest)
        self.map_dir = dest
        return dest

    def add_checks(self, toml: str) -> Path:
        """Append ``[[check]]`` tables to a copy of the fixture map (made on the first call)."""
        mapdir = self.map_dir if self.map_dir != FIXTURE_MAP else self.copy_map()
        with (mapdir / "checks.toml").open("a", encoding="utf-8") as fh:
            fh.write("\n" + toml)
        return mapdir


def unit_toml(check_id: str, cmd: list[str], **fields: object) -> str:
    """One ``[[check]]`` table of a fast unit on L1-L2:fx-alpha (``fields`` add or override keys)."""
    import json
    base: dict[str, object] = {"id": check_id, "kind": "unit", "cmd": cmd, "tier": "fast", "layers": [1],
                               "seams": ["L1-L2:fx-alpha"], "strength": "I", "timeout_s": 30}
    base.update(fields)
    def value(v: object) -> str:
        if isinstance(v, dict):
            return "{ " + ", ".join(f"{json.dumps(k)} = {json.dumps(x)}" for k, x in v.items()) + " }"
        return json.dumps(v)

    lines = ["[[check]]"] + [f"{key} = {value(v)}" for key, v in base.items()]
    return "\n".join(lines) + "\n"


def by_id(records: list[dict]) -> dict[str, dict]:
    return {r["id"]: r for r in records if r.get("kind") == "check"}
