"""Unit tests of record, summary, adapters, locks, run and hygiene (in process, hermetic)."""

from __future__ import annotations

import fcntl
import os
import shutil
import signal
import socket
import subprocess
import tempfile
import threading
import time
import unittest
from pathlib import Path

from t111 import adapters, hygiene, locks, record, summary
from t111.adapters import AdapterContext
from t111.record import Record
from t111.run import Paths, run_check
from t111.schema import Check, Known, load_map

from .helpers import FIXTURE_MAP

MAP = load_map(FIXTURE_MAP)


def tmpdir(test: unittest.TestCase) -> Path:
    path = Path(tempfile.mkdtemp(prefix="t111-unit-"))
    test.addCleanup(shutil.rmtree, path, True)
    return path


def check(**kw: object) -> Check:
    base: dict = dict(id="u.t", kind="unit", cmd=("sh", "-c", "true"), tier="fast", layers=(1,), strength="I",
                      timeout_s=10)
    base.update(kw)
    return Check(**base)


class Records(unittest.TestCase):
    def test_enrich_plain_and_fake(self) -> None:
        rec = record.enrich(Record(id="fx.pass", status="PASS"), MAP.check("fx.pass"), MAP)  # type: ignore[arg-type]
        self.assertEqual((rec.read_first, rec.fake, rec.layers), ("a", None, [1, 2]))
        rec = record.enrich(Record(id="fx.fail", status="FAIL"), MAP.check("fx.fail"), MAP)  # type: ignore[arg-type]
        self.assertEqual(rec.read_first, "a")
        self.assertEqual(rec.fake, {"path": "fx/fake-daemon.py", "mirrors": "b"})
        self.assertEqual(rec.spec, ["fx/SPEC.md §2", "fx/API.md"])
        self.assertEqual(rec.rerun, "sh fx/fail.sh")
        # a strength other than F reads the seam's default side, fake or not
        rec = record.enrich(Record(id="fx.requires", status="FAIL"), MAP.check("fx.requires"), MAP)  # type: ignore
        self.assertEqual((rec.read_first, rec.fake), ("b", None))

    def test_enrich_unit_named_by_a_gate_adapter(self) -> None:
        rec = record.enrich(Record(id="fx.gate/x", status="FAIL", check="fx.covered", test="t1"),
                            MAP.check("fx.gate"), MAP)  # type: ignore[arg-type]
        self.assertEqual(rec.check, "fx.covered")
        self.assertEqual(rec.seams, ["L3-L4:fx-faked"])

    def test_render_keeps_unknown_placeholders(self) -> None:
        self.assertEqual(record.render("cargo test -- --exact {test} {other}", {"test": "a::b", "id": None}),
                         "cargo test -- --exact a::b {other}")

    def test_mark_after(self) -> None:
        recs = [Record("c/1", "PASS", check="c"), Record("c/2", "FAIL", check="c"), Record("c/3", "FAIL", check="c"),
                Record("d/1", "FAIL", check="d"), Record("c/4", "FAIL", check="c", after="c/3")]
        record.mark_after(recs)
        self.assertEqual([r.after for r in recs], [None, None, "c/2", None, "c/3"])

    def test_known_rule(self) -> None:
        known = [Known(test="x/*", condition="euid0", source="S:1"),
                 Known(test="v", condition="euid0", source="S:2", kind="vacuous")]
        root = {"euid0": True}
        fail = record.apply_known(Record("x/a", "FAIL", assertion="boom"), known, root)
        self.assertEqual((fail.status, fail.reason, fail.assertion), ("SKIPPED", "known: fails as root (S:1)", "boom"))
        self.assertEqual(record.apply_known(Record("x/a", "PASS"), known, root).status, "PASS", "passing counts PASS")
        self.assertEqual(record.apply_known(Record("x/a", "FAIL"), known, {"euid0": False}).status, "FAIL")
        vac = record.apply_known(Record("v", "PASS"), known, root)
        self.assertEqual(vac.status, "SKIPPED")
        self.assertTrue(vac.reason.startswith("known: returns early as root"))  # type: ignore[union-attr]
        self.assertEqual(record.apply_require(vac).status, "FAIL")

    def test_clip(self) -> None:
        text = "é" * 2000
        clipped = record.clip(text)
        assert clipped is not None
        self.assertLessEqual(len(clipped.encode()), record.ASSERTION_MAX_BYTES)
        self.assertTrue(text.startswith(clipped))

    def test_record_keys(self) -> None:
        keys = list(Record("a", "PASS").to_json())
        self.assertEqual(keys[:8], ["v", "kind", "id", "check", "seams", "layers", "status", "reason"])
        self.assertIn("after", keys)


class Summary(unittest.TestCase):
    def test_last_lines(self) -> None:
        self.assertEqual(summary.last_line(0, 0), ("VERIFY GREEN", 0))
        self.assertEqual(summary.last_line(0, 3), ("VERIFY GREEN (3 SKIPPED — not a full run)", 0))
        self.assertEqual(summary.last_line(2, 3), ("VERIFY RED (2 FAIL)", 1))
        self.assertEqual(summary.duration(372), "6m12s")
        self.assertEqual(summary.duration(5.4), "5s")

    def test_render_form(self) -> None:
        fail = Record("l1.node111/api_v1_relay/t", "FAIL", assertion="panicked at x\nmore", log_line=5121,
                      seams=["L3-L4:chat-relay"], layers=[4, 3], spec=["4-interface/README.md §10"],
                      sides={"a": {"layer": 4, "files": ["relay.rs"]}, "b": {"layer": 3, "files": ["http.rs"]}},
                      read_first="a", fake={"path": "f.rs", "mirrors": "b"}, rerun="cargo test t",
                      log="D/l1.gate/out.log")
        skip = Record("l7.g11", "SKIPPED", reason="T0 stub (7-relay/TASKS.md V5)", seams=["L2-L7:epoch-linkkey"])
        text, code = summary.render([Record("p", "PASS"), fail, skip], n_checks=14, tiers=("fast", "slow"),
                                    duration_s=372, results="D/results.jsonl")
        self.assertEqual(code, 1)
        self.assertEqual(text.splitlines(), [
            "verify: 1 PASS, 1 FAIL, 1 SKIPPED — 14 checks, fast+slow, 6m12s — D/results.jsonl",
            "FAIL L3-L4:chat-relay [L3↔L4] l1.node111/api_v1_relay/t: panicked at x",
            "     read first: relay.rs (a, under test; b is a fake) · spec 4-interface/README.md §10",
            "     rerun: cargo test t · log: D/l1.gate/out.log:5121",
            "SKIPPED L2-L7:epoch-linkkey l7.g11: T0 stub (7-relay/TASKS.md V5)",
            "VERIFY RED (1 FAIL)",
        ])


class Adapters(unittest.TestCase):
    def ctx(self, d: Path, timed_out: bool = False) -> AdapterContext:
        return AdapterContext(root=d, scratch=d, tmp=d, results=d / "emit.jsonl", timed_out=timed_out, seam_map=MAP)

    def test_exit_adapter(self) -> None:
        d = tmpdir(self)
        log = d / "out.log"
        log.write_text("".join(f"line {i}\n" for i in range(1, 101)))
        self.assertIs(adapters.get("exit"), adapters.exit_adapter)
        ok = adapters.exit_adapter(check(), log, 0, self.ctx(d))
        self.assertEqual([(r.id, r.status) for r in ok], [("u.t", "PASS")])
        bad = adapters.exit_adapter(check(), log, 2, self.ctx(d))[0]
        self.assertEqual((bad.status, bad.reason, bad.log_line), ("FAIL", "exit 2", 61))
        self.assertEqual(bad.assertion.splitlines(), [f"line {i}" for i in range(61, 101)])  # type: ignore[union-attr]
        with self.assertRaises(ValueError):
            adapters.register("exit")(adapters.exit_adapter)


class LockFiles(unittest.TestCase):
    def test_slots_bounds(self) -> None:
        d = tmpdir(self)
        for bad in (0, 3):
            with self.assertRaises(ValueError):
                locks.Locks(d, bad)
        self.assertEqual(locks.lock_dir({"T111_LOCKS": str(d)}), d)
        self.assertEqual(str(locks.lock_dir({})), "/tmp/111-verify-locks")

    def test_named_lock_excludes(self) -> None:
        d = tmpdir(self)
        lk = locks.Locks(d, 1)
        with lk.for_check(("verify",), cargo=True) as held:
            self.assertEqual(held, ["verify", "cargo-slot-0"])
            for name in ("verify.lock", "cargo-slot-0.lock"):
                fd = os.open(d / name, os.O_RDWR)
                try:
                    with self.assertRaises(BlockingIOError):
                        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                finally:
                    os.close(fd)
        fd = os.open(d / "verify.lock", os.O_RDWR)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        os.close(fd)

    def test_two_slots_third_waits(self) -> None:
        d = tmpdir(self)
        lk = locks.Locks(d, 2)
        got: list[int] = []
        with lk.cargo_slot() as a, lk.cargo_slot() as b:
            self.assertEqual({a, b}, {0, 1})
            t = threading.Thread(target=lambda: got.append(lk.cargo_slot().__enter__()), daemon=True)
            t.start()
            t.join(0.3)
            self.assertEqual(got, [], "a third holder waits")
        t.join(5)
        self.assertEqual(len(got), 1)


class RunOne(unittest.TestCase):
    def test_env_tokens_and_log(self) -> None:
        d = tmpdir(self)
        paths = Paths.under(d / "run", "u.t")
        c = check(cmd=("sh", "-c", 'echo "$TMPDIR|$T111_SCRATCH|$T111_RUN|$T111_CHECK|$X|$1" >&2', "_", "{scratch}"),
                  env={"X": "{root}/x"})
        out = run_check(c, root=d, paths=paths, run_id="R", run_dir=d / "run")
        self.assertEqual((out.status, out.timed_out), (0, False))
        self.assertEqual(paths.log.read_text().strip(),
                         f"{paths.tmp}|{paths.scratch}|R|u.t|{d}/x|{paths.scratch}")

    def test_term_ignored_needs_kill(self) -> None:
        d = tmpdir(self)
        paths = Paths.under(d / "run", "u.t")
        c = check(cmd=("sh", "-c", "trap '' TERM; echo up; sleep 30 & wait"), timeout_s=1)
        began = time.monotonic()
        out = run_check(c, root=d, paths=paths, run_id="R", run_dir=d / "run", kill_grace_s=0.5)
        self.assertLess(time.monotonic() - began, 10)
        self.assertTrue(out.timed_out)
        self.assertTrue(out.killed)
        self.assertEqual(out.status, -signal.SIGKILL)

    def test_cannot_start(self) -> None:
        d = tmpdir(self)
        paths = Paths.under(d / "run", "u.t")
        out = run_check(check(cmd=("/nonexistent/t111-binary",)), root=d, paths=paths, run_id="R", run_dir=d / "run")
        self.assertEqual(out.status, 127)
        self.assertIsNotNone(out.error)


class HygieneUnits(unittest.TestCase):
    def test_tree_state_and_diff(self) -> None:
        d = tmpdir(self)
        subprocess.run(["git", "init", "-q", "."], cwd=d, check=True, timeout=30)
        (d / "a.txt").write_text("1")
        s0 = hygiene.tree_state(d)
        (d / "a.txt").write_text("2")  # still "??", content changed
        (d / "b.txt").write_text("x")
        s1 = hygiene.tree_state(d)
        self.assertEqual(hygiene.tree_diff(s0, s1), ["a.txt", "b.txt"])
        self.assertEqual(hygiene.tree_diff(s1, s1), [])
        self.assertIsNone(hygiene.tree_state(Path("/proc")))

    def test_port_holders(self) -> None:
        sock = socket.socket()
        self.addCleanup(sock.close)
        sock.bind(("127.0.0.1", 0))
        sock.listen(1)
        port = sock.getsockname()[1]
        held = hygiene.port_holders([port])
        self.assertEqual(held[port][0], os.getpid())
        self.assertIn(port, hygiene.ports_of(os.getpid()))

    def test_find_leftovers_by_marker_and_scratch(self) -> None:
        d = tmpdir(self)
        env = dict(os.environ, T111_RUN="RUN-x", T111_CHECK="c.one")
        marked = subprocess.Popen(["sleep", "60"], env=env, start_new_session=True)
        named = subprocess.Popen(["sleep", "61"], env=dict(os.environ, WORKDIR=str(d)), start_new_session=True)
        other = subprocess.Popen(["sleep", "62"], env=dict(os.environ, T111_RUN="RUN-x", T111_CHECK="c.live"),
                                 start_new_session=True)
        for p in (marked, named, other):
            self.addCleanup(p.wait, 10)
            self.addCleanup(p.kill)
        time.sleep(0.2)
        found = hygiene.find_leftovers(runner_pid=-1, run_id="RUN-x", finished="c.one", scratch=d,
                                       live={"c.live": other.pid}, before=set())
        pids = {lo.proc.pid: lo for lo in found}
        self.assertIn(marked.pid, pids, "marker rule")
        self.assertEqual(pids[marked.pid].owner, "c.one")
        self.assertIn(named.pid, pids, "scratch rule")
        self.assertNotIn(other.pid, pids, "a live check's process is not a leftover")
        self.assertTrue(pids[marked.pid].kill, "the marker is proof: killed")
        # X2c C6: naming the scratch dir alone is no proof (an agent's tail -f): reported, never killed
        self.assertFalse(pids[named.pid].kill)
        self.assertIn("[not killed: it names the check's scratch dir", pids[named.pid].describe())

    def test_is_ours(self) -> None:
        P = hygiene.Proc
        mk = (lambda argv: P(1, 0, 1, 1, "S", 0, argv, {}))
        self.assertTrue(hygiene.is_ours(mk(["/x/111", "daemon"])))
        self.assertTrue(hygiene.is_ours(mk(["python3", "/v/stub-llama.py"])))
        self.assertTrue(hygiene.is_ours(mk(["chromium", "--headless=new"])))
        self.assertTrue(hygiene.is_ours(mk(["node", "/ui/node_modules/.bin/vite"])))
        self.assertFalse(hygiene.is_ours(mk(["bash"])))

    def test_probes(self) -> None:
        pr = hygiene.Probes(offline=True, run_id="R", cwd=Path("/"))
        self.assertFalse(pr.has("network"))
        self.assertTrue(pr.has("sh"))
        self.assertEqual(pr.missing(["sh", "t111-no-such-tool"]), ["t111-no-such-tool"])
        self.assertEqual(pr.has("root"), os.geteuid() == 0)
        self.assertIn("network", pr.found())


if __name__ == "__main__":
    unittest.main()
