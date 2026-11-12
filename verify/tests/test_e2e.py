"""End to end: ``verify/run.py`` as its own process against the fixture map and a temp git tree."""

from __future__ import annotations

import os
import re
import socket
import subprocess
import time
import unittest
from pathlib import Path

from . import helpers
from .helpers import Sandbox, by_id, max_overlap


def last_line(text: str) -> str:
    return text.rstrip("\n").splitlines()[-1]


def procs_with_marker(check_id: str) -> list[int]:
    """Pids whose environ says ``T111_CHECK=<check_id>`` (any run)."""
    found = []
    needle = f"T111_CHECK={check_id}".encode()
    for entry in os.listdir("/proc"):
        if not entry.isdigit():
            continue
        try:
            with open(f"/proc/{entry}/environ", "rb") as fh:
                env = fh.read().split(b"\0")
            with open(f"/proc/{entry}/stat", "rb") as fh:
                state = fh.read().rsplit(b")", 1)[1].split()[0]
        except OSError:
            continue
        if needle in env and state != b"Z":
            found.append(int(entry))
    return found


class Basics(unittest.TestCase):
    def test_pass(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.pass")
        self.assertEqual(done.returncode, 0, done.stderr + done.stdout)
        self.assertEqual(last_line(done.stdout), "VERIFY GREEN")
        rows = box.results()
        self.assertEqual(rows[0]["kind"], "run")
        self.assertEqual(rows[0]["checks"], ["fx.pass"])
        rec = by_id(rows)["fx.pass"]
        self.assertEqual(rec["status"], "PASS")
        self.assertEqual(rec["seams"], ["L1-L2:fx-alpha"])
        self.assertEqual(rec["layers"], [1, 2])
        self.assertEqual(rec["spec"], ["fx/SPEC.md §1"])
        self.assertEqual(rec["sides"]["a"], {"layer": 1, "files": ["fx/a.txt"]})
        self.assertEqual(rec["read_first"], "a")
        self.assertEqual(rec["rerun"], "sh fx/pass.sh  # fx.pass")
        self.assertEqual(rec["rerun_cost"], "fast, ~0 s")
        self.assertIsNone(rec["log"], "a green check's scratch is deleted")
        run_dir = box.run_dirs()[-1]
        self.assertFalse((run_dir / "fx.pass").exists())
        self.assertEqual((run_dir / "summary.txt").read_text(), done.stdout)
        self.assertRegex(done.stdout.splitlines()[0],
                         r"^verify: 1 PASS, 0 FAIL, 0 SKIPPED — 1 checks, fast, \d+s — .*/results\.jsonl$")

    def test_keep(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.pass", "--keep")
        self.assertEqual(done.returncode, 0)
        rec = by_id(box.results())["fx.pass"]
        self.assertTrue(Path(rec["log"]).is_file())
        self.assertIn("ok: the fixture passes", Path(rec["log"]).read_text())

    def test_fail(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.fail")
        self.assertEqual(done.returncode, 1, done.stderr)
        self.assertEqual(last_line(done.stdout), "VERIFY RED (1 FAIL)")
        rec = by_id(box.results())["fx.fail"]
        self.assertEqual(rec["status"], "FAIL")
        self.assertEqual(rec["reason"], "exit 3")
        self.assertIn("FAIL: the fixture says no  [HTTP 502: upstream]", rec["assertion"])
        self.assertEqual(rec["fake"], {"path": "fx/fake-daemon.py", "mirrors": "b"})
        self.assertEqual(rec["read_first"], "a", "against a fake, read the side under test")
        self.assertEqual(rec["log_line"], 1)
        self.assertTrue(Path(rec["log"]).is_file(), "a red check's scratch is kept")
        # stdout and stderr in one ordered stream
        self.assertEqual(Path(rec["log"]).read_text().splitlines()[:2],
                         ["ok: first line", "FAIL: the fixture says no  [HTTP 502: upstream]"])
        lines = done.stdout.splitlines()
        self.assertEqual(lines[1], 'note: 1 fake(s) checked against nothing they mirror (parity = "none")')
        self.assertEqual(lines[2], "FAIL L3-L4:fx-faked [L3↔L4] fx.fail: exit 3: FAIL: the fixture says no  "
                                   "[HTTP 502: upstream]")
        self.assertEqual(lines[3], "     read first: fx/agent.txt (a, under test; b is a fake) · spec fx/SPEC.md §2, "
                                   "fx/API.md")
        self.assertTrue(lines[4].startswith("     rerun: sh fx/fail.sh · log: "), lines[4])
        self.assertTrue(lines[4].endswith("/fx.fail/out.log:1"), lines[4])

    def test_needs_skip_and_require(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.needs")
        self.assertEqual(done.returncode, 0)
        self.assertEqual(last_line(done.stdout), "VERIFY GREEN (1 SKIPPED — not a full run)")
        rec = by_id(box.results())["fx.needs"]
        self.assertEqual((rec["status"], rec["reason"]), ("SKIPPED", "needs t111-no-such-tool-anywhere (missing)"))
        self.assertIn("SKIPPED L1-L2:fx-alpha fx.needs: needs t111-no-such-tool-anywhere (missing)",
                      done.stdout.splitlines())
        req = box.run("--check", "fx.needs", "--require")
        self.assertEqual(req.returncode, 1)
        self.assertEqual(last_line(req.stdout), "VERIFY RED (1 FAIL)")
        self.assertEqual(by_id(box.results())["fx.needs"]["status"], "FAIL")

    def test_offline(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.offline", "--offline")
        self.assertEqual(done.returncode, 0)
        rec = by_id(box.results())["fx.offline"]
        self.assertEqual((rec["status"], rec["reason"]), ("SKIPPED", "offline"))
        online = box.run("--check", "fx.offline")
        self.assertEqual(online.returncode, 0)
        self.assertEqual(by_id(box.results())["fx.offline"]["status"], "PASS")


class Hygiene(unittest.TestCase):
    def test_hang_killed_at_timeout(self) -> None:
        box = Sandbox(self)
        began = time.monotonic()
        done = box.run("--check", "fx.hang", "--kill-grace", "2")
        self.assertLess(time.monotonic() - began, 20)
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        rec = by_id(box.results())["fx.hang"]
        self.assertEqual(rec["status"], "FAIL")
        self.assertTrue(rec["reason"].startswith("timeout after 1 s (TERM to the group, then KILL after 2 s)"),
                        rec["reason"])
        self.assertIn("started, now hanging", rec["assertion"])
        self.assertEqual(procs_with_marker("fx.hang"), [])

    def test_leftover_caught_and_killed(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.leak")
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        recs = by_id(box.results())
        self.assertEqual(recs["fx.leak"]["status"], "PASS")
        hyg = recs["hygiene/fx.leak"]
        self.assertEqual(hyg["status"], "FAIL")
        self.assertIn("sleep 300", hyg["reason"])
        self.assertIn("sleep 301", hyg["reason"], "a setsid'd leftover is caught too")
        self.assertRegex(hyg["reason"], r"pid \d+ \(sleep 300\)")
        self.assertEqual(hyg["check"], "fx.leak")
        self.assertEqual(procs_with_marker("fx.leak"), [], "the leftovers were killed")

    def test_tree_dirtied(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.dirty")
        self.assertEqual(done.returncode, 1)
        rec = by_id(box.results())["hygiene/fx.dirty/tree"]
        self.assertEqual(rec["status"], "FAIL")
        self.assertIn("fx-dirty.txt", rec["reason"])
        # a check that writes only its scratch leaves the tree alone
        box2 = Sandbox(self)
        self.assertEqual(box2.run("--check", "fx.writer").returncode, 0)

    def test_nothing_written_outside_scratch(self) -> None:
        box = Sandbox(self)

        def files() -> dict[str, tuple[int, int]]:
            out = {}
            for path in box.base.rglob("*"):
                if box.scratch in path.parents or path == box.scratch:
                    continue
                st = path.lstat()
                out[str(path)] = (st.st_mtime_ns, st.st_size)
            return out

        before = files()
        done = box.run("--check", "fx.pass,fx.fail,fx.writer,fx.needs", "--keep")
        self.assertEqual(done.returncode, 1)
        self.assertEqual(files(), before)
        status = subprocess.run(["git", "status", "--porcelain", "--untracked-files=all"], cwd=box.tree, text=True,
                                capture_output=True, timeout=30, check=True)
        self.assertEqual(status.stdout, "")
        run_dir = box.run_dirs()[-1]
        self.assertTrue((run_dir / "fx.writer" / "own.txt").is_file())
        self.assertTrue((run_dir / "fx.writer" / "tmp" / "t.txt").is_file())
        self.assertTrue(any((run_dir / "fx.writer" / "tmp").glob("tmp.*")), "mktemp landed in the check's TMPDIR")

    def test_ns_port_held(self) -> None:
        box = Sandbox(self)
        sock = socket.socket()
        self.addCleanup(sock.close)
        sock.bind(("127.0.0.1", 0))
        sock.listen(1)
        port = sock.getsockname()[1]
        done = box.run("--tier", "ns", "--check", "fx.ns", "--ns-ports", str(port))
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        rec = by_id(box.results())["fx.ns"]
        self.assertEqual(rec["status"], "SKIPPED")
        self.assertTrue(rec["reason"].startswith(f"port {port} held by pid {os.getpid()} ("), rec["reason"])
        sock.close()
        free = box.run("--tier", "ns", "--check", "fx.ns", "--ns-ports", str(port))
        self.assertEqual(by_id(box.results())["fx.ns"]["status"], "PASS", free.stdout)


class Ordering(unittest.TestCase):
    def test_requires(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.requires,fx.after-pass")
        self.assertEqual(done.returncode, 1)
        rows = box.results()
        self.assertEqual(rows[0]["checks"], ["fx.pass", "fx.fail", "fx.requires", "fx.after-pass"])
        self.assertEqual(sorted(rows[0]["pulled"]), ["fx.fail", "fx.pass"])
        recs = by_id(rows)
        self.assertEqual(recs["fx.requires"]["status"], "SKIPPED")
        self.assertEqual(recs["fx.requires"]["reason"], "requires fx.fail (FAIL)")
        self.assertEqual(recs["fx.after-pass"]["status"], "PASS")

    def test_known(self) -> None:
        box = Sandbox(self)
        mapdir = box.copy_map()
        holds = "euid0" if os.geteuid() == 0 else "nonroot"
        text = (mapdir / "checks.toml").read_text().replace('condition = "euid0"', f'condition = "{holds}"')
        (mapdir / "checks.toml").write_text(text)
        done = box.run("--check", "fx.known")
        self.assertEqual(done.returncode, 0, done.stdout)
        rec = by_id(box.results())["fx.known"]
        self.assertEqual(rec["status"], "SKIPPED")
        self.assertEqual(rec["reason"], "known: fails as root in the fixture (fx/TASKS.md:1)")
        req = box.run("--check", "fx.known", "--require")
        self.assertEqual(req.returncode, 1)
        self.assertEqual(by_id(box.results())["fx.known"]["status"], "FAIL")
        other = "nonroot" if holds == "euid0" else "euid0"
        (mapdir / "checks.toml").write_text(text.replace(f'condition = "{holds}"', f'condition = "{other}"'))
        self.assertEqual(box.run("--check", "fx.known").returncode, 1, "the condition does not hold: FAIL stands")


class Locks(unittest.TestCase):
    def test_one_cargo_slot_never_overlaps(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.cargo1,fx.cargo2", "--cargo-slots", "1", "--jobs", "2")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(max_overlap(box.events), 1)

    def test_two_cargo_slots_never_more_than_two(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.cargo1,fx.cargo2,fx.cargo3,fx.cargo4", "--cargo-slots", "2", "--jobs", "4")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(len(box.events.read_text().splitlines()), 8)
        self.assertEqual(max_overlap(box.events), 2, "two at once, never three")

    def test_slots_shared_between_instances(self) -> None:
        box = Sandbox(self)
        procs = [subprocess.Popen(box.argv("--check", check, "--cargo-slots", "1"), cwd=box.base, env=box.env(),
                                  stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
                 for check in ("fx.cargo1", "fx.cargo2")]
        self.assertEqual([p.wait(timeout=60) for p in procs], [0, 0])
        self.assertEqual(max_overlap(box.events), 1)

    def test_verify_lock_serial(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.verify1,fx.verify2", "--jobs", "2")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        self.assertEqual(max_overlap(box.events), 1)
        control = Sandbox(self)
        self.assertEqual(control.run("--check", "fx.free1,fx.free2", "--jobs", "2").returncode, 0)
        self.assertEqual(max_overlap(control.events), 2, "positive control: unlocked checks do overlap")


class Commands(unittest.TestCase):
    def test_list_runs_nothing(self) -> None:
        box = Sandbox(self)
        done = box.run("--list", "--check", "fx.cargo1,fx.verify1,fx.requires")
        self.assertEqual(done.returncode, 0, done.stderr)
        self.assertFalse(box.events.exists())
        self.assertFalse(box.scratch.exists(), "--list creates no run dir")
        out = done.stdout
        self.assertIn("nothing was run", out)
        self.assertRegex(out, r"fx\.cargo1\s+unit\s+slow\s+cargo-slot")
        self.assertRegex(out, r"fx\.verify1\s+unit\s+fast\s+verify")
        self.assertIn("fx.fail (pulled in)", out)
        # an estimate from the last run under --scratch
        box.run("--check", "fx.pass")
        listed = box.run("--list", "--check", "fx.pass")
        self.assertRegex(listed.stdout, r"fx\.pass .*~\d+s \(last run\)")

    def test_usage_errors_exit_2(self) -> None:
        box = Sandbox(self)
        self.assertEqual(box.run("--check", "fx.nope").returncode, 2)
        self.assertEqual(box.run("--seam", "L1-L2:nope").returncode, 2)
        self.assertEqual(box.run("--tier", "medium").returncode, 2)
        self.assertEqual(box.run("--check", "fx.pass", scratch=False).returncode, 2)
        self.assertEqual(box.run("--check", "fx.pass", "--cargo-slots", "3").returncode, 2)
        self.assertEqual(box.run("--list", "--lint").returncode, 2)
        in_tree = box.tree / "scratch-inside"
        inside = box.run("--check", "fx.pass", "--scratch", str(in_tree), scratch=False)
        self.assertEqual(inside.returncode, 2, inside.stderr)
        self.assertIn("--scratch must be outside the tree", inside.stderr)
        self.assertFalse(in_tree.exists(), "a refused --scratch is not created")
        bad = box.copy_map()
        (bad / "checks.toml").write_text('[[check]]\nid = "x"\nkind = "unit"\ntier = "medium"\n')
        broken = box.run("--check", "x")
        self.assertEqual(broken.returncode, 2)
        self.assertIn("check[0] (x).tier: 'medium' is not one of fast|slow|ns", broken.stderr)
        self.assertFalse(box.events.exists())

    def test_x4_commands_guarded(self) -> None:
        box = Sandbox(self)
        for flag, module in (("--lint", "lint"), ("--matrix", "matrix"), ("--changed", "changed")):
            if (helpers.VERIFY / "t111" / f"{module}.py").exists():
                continue
            done = box.run(flag, scratch=False)
            self.assertEqual(done.returncode, 2)
            self.assertIn("not built yet (row X4)", done.stderr)
        if not (helpers.VERIFY / "t111" / "explain.py").exists():
            done = box.run("--explain", "fx.pass", scratch=False)
            self.assertEqual((done.returncode, "not built yet (row X4)" in done.stderr), (2, True))

    def test_green_with_skips_and_red(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.pass,fx.needs")
        self.assertEqual((done.returncode, last_line(done.stdout)), (0, "VERIFY GREEN (1 SKIPPED — not a full run)"))
        self.assertTrue(re.match(r"verify: 1 PASS, 0 FAIL, 1 SKIPPED — 2 checks", done.stdout))


if __name__ == "__main__":
    unittest.main()
