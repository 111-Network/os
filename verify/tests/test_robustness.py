"""The X2 review's findings, one class per finding (F1–F12 of the X2b brief), each pinned by a test.

Every check is a tiny script in the sandbox tree (``helpers.FX_SCRIPTS``) or an inline ``sh -c``; checks this file adds
are appended to a copy of the fixture map (``Sandbox.add_checks``). Waits poll a condition with a deadline: no pass or
fail decision reads a duration.
"""

from __future__ import annotations

import fcntl
import io
import json
import os
import shlex
import shutil
import signal
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path
from unittest import mock

from t111 import adapters, engine, hygiene, locks, record, summary
from t111.adapters import AdapterContext
from t111.record import Record, RerunPaths
from t111.run import Outcome, Paths, run_check
from t111.schema import Check, Known, load_map
from t111.select import Selection, select

from .helpers import FIXTURE_MAP, Sandbox, by_id, unit_toml

MAP = load_map(FIXTURE_MAP)

DEFAULT_SIGNALS = ("import os, signal, sys\n"
                   "for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):\n"
                   "    signal.signal(s, signal.SIG_DFL)\n"
                   "os.execv(sys.argv[1], sys.argv[1:])\n")
"""Exec the runner with the stop signals at their defaults (a test process may have inherited SIGINT ignored)."""


def tmpdir(test: unittest.TestCase) -> Path:
    path = Path(tempfile.mkdtemp(prefix="t111-x2b-"))
    test.addCleanup(shutil.rmtree, path, True)
    return path


def last_line(text: str) -> str:
    return text.rstrip("\n").splitlines()[-1]


def wait_for(cond, what: str, limit_s: float = 30.0) -> None:
    """Poll ``cond`` until it holds; fail the test naming ``what`` after ``limit_s`` (a bound, not a verdict)."""
    deadline = time.monotonic() + limit_s
    while not cond():
        if time.monotonic() > deadline:
            raise AssertionError(f"timed out waiting for {what}")
        time.sleep(0.05)


def marked(check_id: str) -> list[hygiene.Proc]:
    """Live (non-zombie) processes whose environ says ``T111_CHECK=<check_id>``."""
    return [p for p in hygiene.snapshot().values()
            if p.state != "Z" and (p.environ or {}).get("T111_CHECK") == check_id]


def kill_marked(check_id: str) -> None:
    for proc in marked(check_id):
        try:
            os.kill(proc.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def check(**kw: object) -> Check:
    base: dict = dict(id="u.t", kind="unit", cmd=("sh", "-c", "true"), tier="fast", layers=(1,), strength="I",
                      timeout_s=10)
    base.update(kw)
    return Check(**base)


def bare_engine(d: Path, **cfg: object) -> engine.Engine:
    """An ``Engine`` with only what ``parse`` and ``hygiene_records`` read (no run dir, no subreaper)."""
    eng = object.__new__(engine.Engine)
    plan = select(MAP, Selection(checks=("fx.pass",)))
    eng.cfg = engine.Config(root=d, scratch=d, seam_map=MAP, plan=plan, **cfg)  # type: ignore[arg-type]
    eng.facts = {"euid0": os.geteuid() == 0, "offline": False}
    eng._stopped_live = set()
    eng._stop_why = ""
    return eng


def ran(d: Path, outcome: Outcome, **kw: object) -> engine.Ran:
    paths = Paths.under(d, "u.t")
    paths.tmp.mkdir(parents=True, exist_ok=True)
    if not paths.log.exists():
        paths.log.write_text("ok: one\nerror: the real cause\n")
    base: dict = dict(paths=paths, outcome=outcome, found=[], changed=[], window=["u.t"], cut=False)
    base.update(kw)
    return engine.Ran(**base)


# ---------------------------------------------------------------------------------------------------------------------


class F1NeverReapAnOwnedPid(unittest.TestCase):
    def test_reap_skips_a_registered_zombie(self) -> None:
        """Deterministic: a registered child is a zombie while every reap runs; its exit status survives."""
        owned = hygiene.SPAWNED.popen(["sh", "-c", "exit 3"], cwd="/", stdin=subprocess.DEVNULL,
                                      start_new_session=True)
        stray = subprocess.Popen(["sh", "-c", "exit 4"], cwd="/", stdin=subprocess.DEVNULL, start_new_session=True)
        self.addCleanup(stray.wait, 10)

        def zombie(pid: int) -> bool:
            now = hygiene.read_proc(pid, with_environ=False)
            return now is not None and now.state == "Z"

        wait_for(lambda: zombie(owned.pid) and zombie(stray.pid), "both children to exit")
        self.assertIn(owned.pid, hygiene.SPAWNED.pids())
        hygiene.reap_orphans(os.getpid())
        hygiene.kill_leftovers([], os.getpid())
        hygiene.reap(owned.pid)
        self.assertTrue(zombie(owned.pid), "a registered pid is never reaped")
        self.assertIsNone(hygiene.read_proc(stray.pid, with_environ=False), "positive control: a stray zombie is")
        self.assertEqual(hygiene.SPAWNED.wait(owned, timeout=10), 3)
        self.assertNotIn(owned.pid, hygiene.SPAWNED.pids())

    def test_failing_check_beside_a_leaking_one_is_never_pass(self) -> None:
        """End to end: with --jobs 2, every check end reaps; a failing check whose leader is a zombie between its
        worker's polls must still be FAIL. (Against the X2 runner this shape turned 1 to 4 of 36 FAILs into PASS.)"""
        box = Sandbox(self)
        quick = [f"fx.q{i}" for i in range(12)]
        fill = [f"fx.f{i}" for i in range(24)]
        leaks = ["fx.leak0", "fx.leak1"]
        box.add_checks("".join(unit_toml(q, ["sh", "fx/quickfail.sh"]) for q in quick)
                       + "".join(unit_toml(k, ["sh", "fx/leak.sh"]) for k in leaks)
                       + "".join(unit_toml(f, ["sh", "fx/pass.sh"]) for f in fill))
        order = leaks + [c for i, q in enumerate(quick) for c in (q, fill[2 * i], fill[2 * i + 1])]
        for _ in range(3):
            done = box.run("--check", ",".join(order), "--jobs", "2", "--ci", timeout=300)
            self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
            recs = by_id(box.results())
            for q in quick:
                self.assertEqual((recs[q]["status"], recs[q]["reason"]), ("FAIL", "exit 3"), q)
            for f in fill:
                self.assertEqual(recs[f]["status"], "PASS", f)
            for k in leaks:
                self.assertEqual(recs[f"hygiene/{k}"]["status"], "FAIL", k)
                self.assertEqual(marked(k), [])


class F2StopSignals(unittest.TestCase):
    def start(self, box: Sandbox, *args: str) -> subprocess.Popen:
        argv = [sys.executable, "-I", "-c", DEFAULT_SIGNALS, *box.argv(*args)]
        proc = subprocess.Popen(argv, cwd=box.base, env=box.env(), text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, start_new_session=True)
        self.addCleanup(lambda: proc.poll() is None and proc.kill())
        return proc

    def long_box(self) -> Sandbox:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.long", ["sh", "fx/long.sh"], timeout_s=120)
                       + unit_toml("fx.later", ["sh", "fx/pass.sh"]))
        self.addCleanup(kill_marked, "fx.long")
        return box

    def test_each_stop_signal_kills_sweeps_and_ends_red(self) -> None:
        for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
            with self.subTest(sig=sig.name):
                box = self.long_box()
                proc = self.start(box, "--check", "fx.long,fx.later", "--kill-grace", "2")
                wait_for(lambda: {"302", "303"} <= {p.argv[-1] for p in marked("fx.long") if p.argv},
                         "fx.long and its two sleepers")
                proc.send_signal(sig)
                out, err = proc.communicate(timeout=90)
                self.assertEqual(proc.returncode, 1, out + err)
                self.assertTrue(last_line(out).startswith("VERIFY RED ("), out)
                self.assertNotIn("Traceback", err)
                recs = by_id(box.results())
                self.assertEqual(recs["fx.long"]["status"], "FAIL")
                self.assertEqual(recs["fx.long"]["reason"], f"interrupted by {sig.name}: the runner killed the check")
                self.assertIn("long: started", recs["fx.long"]["assertion"])
                self.assertEqual((recs["fx.later"]["status"], recs["fx.later"]["reason"]),
                                 ("SKIPPED", f"not started: interrupted by {sig.name}"))
                self.assertEqual(recs["runner"]["status"], "FAIL")
                self.assertEqual(recs["runner"]["reason"], f"interrupted by {sig.name}: 1 check(s) killed, "
                                                           "1 not started")
                self.assertEqual(marked("fx.long"), [], "the setsid'd sleeper is gone too")
                self.assertIn("left running", recs["hygiene/fx.long"]["reason"])

    def test_dead_run_swept_at_the_next_start(self) -> None:
        box = self.long_box()
        proc = self.start(box, "--check", "fx.long")
        wait_for(lambda: len(marked("fx.long")) >= 3, "fx.long and its sleepers")
        proc.kill()
        proc.communicate(timeout=30)
        dead = box.run_dirs()[0].name
        survivors = marked("fx.long")
        self.assertGreaterEqual(len(survivors), 3, "a SIGKILLed runner leaves its check running")
        self.assertTrue(all((p.environ or {}).get("T111_RUNNER_PID") == str(proc.pid) for p in survivors))
        done = box.run("--check", "fx.pass")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        note = f"note: killed {len(survivors)} leftover process(es) of dead run {dead}"
        self.assertIn(note, done.stdout.splitlines(), "one summary line, a note and not a FAIL")
        self.assertIn(f"t111: {note}", done.stderr)
        self.assertEqual(last_line(done.stdout), "VERIFY GREEN")
        self.assertEqual(marked("fx.long"), [])
        self.assertEqual(box.results()[0]["swept"], {dead: len(survivors)})

    def test_unmarked_and_live_runs_are_never_swept(self) -> None:
        env = dict(os.environ, T111_RUN="RUN-other", T111_CHECK="c.x")
        for key in ("T111_RUNNER_PID", "T111_RUNNER_START"):
            env.pop(key, None)
        no_runner = subprocess.Popen(["sleep", "30"], env=env, start_new_session=True)
        live = subprocess.Popen(["sleep", "31"], env=dict(env, T111_RUNNER_PID=str(os.getpid())),
                                start_new_session=True)
        dead = subprocess.Popen(["sleep", "32"], env=dict(env, T111_RUN="RUN-dead", T111_RUNNER_PID="999999999"),
                                start_new_session=True)
        for p in (no_runner, live, dead):
            self.addCleanup(p.wait, 10)
            self.addCleanup(p.kill)
        wait_for(lambda: all(hygiene.read_proc(p.pid) and (hygiene.read_proc(p.pid).environ or {}).get("T111_RUN")
                             for p in (no_runner, live, dead)), "the three sleepers to exec")
        found = hygiene.sweep_dead_runs("RUN-mine")
        pids = {p.pid for procs in found.values() for p in procs}
        self.assertIn(dead.pid, [p.pid for p in found.get("RUN-dead", [])])
        self.assertNotIn("RUN-other", found)
        self.assertNotIn(no_runner.pid, pids, "no runner marker: never touched")
        self.assertNotIn(live.pid, pids, "a live runner's process: never touched")

    def test_lock_wait_is_bounded_and_names_the_holder(self) -> None:
        box = Sandbox(self)
        box.locks.mkdir(parents=True)
        fd = os.open(box.locks / "verify.lock", os.O_RDWR | os.O_CREAT, 0o666)
        self.addCleanup(os.close, fd)
        fcntl.flock(fd, fcntl.LOCK_EX)
        os.write(fd, b"run=RUN-holder pid=4242\n")
        done = box.run("--check", "fx.verify1", "--lock-wait", "1")
        self.assertEqual(done.returncode, 1, done.stdout + done.stderr)
        rec = by_id(box.results())["fx.verify1"]
        self.assertEqual(rec["reason"], "lock verify not free after 1 s (held by run RUN-holder, pid 4242)")
        self.assertIn("t111: fx.verify1 waiting for verify held by run RUN-holder (pid 4242)", done.stderr)
        self.assertEqual(done.stderr.count("waiting for"), 1, "one progress line")

    def test_lock_lives_with_the_check(self) -> None:
        d = tmpdir(self)
        lk = locks.Locks(d / "locks", run_id="RUN-l", pid=777)
        fd = lk.take_named("verify")
        self.assertEqual(locks.read_holder(d / "locks" / "verify.lock"), ("RUN-l", "777"))
        c = check(id="u.lockheld", cmd=("sh", "-c", "setsid sleep 30 >/dev/null 2>&1 &"))
        self.addCleanup(kill_marked, "u.lockheld")
        out = run_check(c, root=d, paths=Paths.under(d / "run", c.id), run_id="RUN-l", run_dir=d / "run",
                        pass_fds=[fd])
        self.assertEqual(out.status, 0)
        lk.release(fd)

        def free() -> bool:
            probe = os.open(d / "locks" / "verify.lock", os.O_RDWR)
            try:
                fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return True
            except BlockingIOError:
                return False
            finally:
                os.close(probe)

        wait_for(lambda: marked("u.lockheld"), "the sleeper")
        self.assertFalse(free(), "the check's leftover still holds the lock after the runner let go")
        kill_marked("u.lockheld")
        wait_for(free, "the lock to come free once the last holder died", 10)


class F3KnownOnlyOnParsedResults(unittest.TestCase):
    def test_engine_and_hygiene_records_are_never_eligible(self) -> None:
        known = [Known(test="*", condition="euid0", source="S:1")]
        facts = {"euid0": True}
        for rec in (Record("x", "FAIL", reason="timeout after 5 s", engine=True),
                    Record("hygiene/x", "FAIL", reason="left running: pid 1 (sleep)"),
                    Record("hygiene/x/tree", "FAIL", reason="the tree changed")):
            self.assertEqual(record.apply_known(rec, known, facts).status, "FAIL", rec.id)
        self.assertEqual(record.apply_known(Record("x/t", "FAIL"), known, facts).status, "SKIPPED")

    def test_after_counts_only_what_is_still_fail(self) -> None:
        recs = [Record("c/1", "FAIL", check="c"), Record("c/2", "FAIL", check="c")]
        record.apply_known(recs[0], [Known(test="c/1", condition="euid0", source="S")], {"euid0": True})
        record.mark_after(recs)
        self.assertEqual([(r.status, r.after) for r in recs], [("SKIPPED", None), ("FAIL", None)])

    def test_a_glob_on_the_check_id_does_not_hide_a_timeout(self) -> None:
        box = Sandbox(self)
        holds = "euid0" if os.geteuid() == 0 else "nonroot"
        box.add_checks(f'[[known]]\ntest = "fx.hang"\ncondition = "{holds}"\nsource = "S:9"\nreason = "flaky"\n')
        done = box.run("--check", "fx.hang", "--kill-grace", "1")
        self.assertEqual(done.returncode, 1, done.stdout)
        rec = by_id(box.results())["fx.hang"]
        self.assertEqual(rec["status"], "FAIL")
        self.assertTrue(rec["reason"].startswith("timeout after 1 s"), rec["reason"])


class F4NonZeroExitBackstop(unittest.TestCase):
    def adapter(self, declared: bool):
        def parse(chk: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
            ctx.exit_declared = declared
            return [Record(f"{chk.id}/a", "PASS"), Record(f"{chk.id}/b", "SKIPPED", reason="ignored test")]
        return parse

    def test_undeclared_exit_fails_beside_pass_and_skipped(self) -> None:
        d = tmpdir(self)
        recs = bare_engine(d).parse(check(), self.adapter(False), ran(d, Outcome(2, False, False, 0.1)))
        self.assertEqual([(r.id, r.status) for r in recs], [("u.t/a", "PASS"), ("u.t/b", "SKIPPED"), ("u.t", "FAIL")])
        self.assertEqual(recs[-1].reason, "exit 2 but no failing result was parsed")
        self.assertTrue(recs[-1].engine)
        self.assertEqual(recs[-1].assertion, "ok: one\nerror: the real cause")

    def test_declared_exit_stands(self) -> None:
        d = tmpdir(self)
        recs = bare_engine(d).parse(check(), self.adapter(True), ran(d, Outcome(77, False, False, 0.1)))
        self.assertEqual([(r.id, r.status) for r in recs], [("u.t/a", "PASS"), ("u.t/b", "SKIPPED")])

    def test_the_contract_is_written_for_the_adapters(self) -> None:
        doc = adapters.__doc__ or ""
        self.assertIn("exit_declared", doc)
        self.assertIn("PASS and SKIPPED records notwithstanding", doc)
        self.assertFalse(AdapterContext(root=Path("/"), scratch=Path("/"), tmp=Path("/"), results=Path("/"),
                                        timed_out=False, seam_map=MAP).exit_declared)


class F5TailFromTheEnd(unittest.TestCase):
    def test_long_lines_keep_the_last_ones(self) -> None:
        d = tmpdir(self)
        log = d / "out.log"
        lines = [f"line {i:03d} " + "." * 90 for i in range(1, 61)] + ["error: the last line, the real one"]
        log.write_text("\n".join(lines) + "\n")
        tail, first = adapters.log_tail(log)
        self.assertLessEqual(len(tail.encode()), record.ASSERTION_MAX_BYTES)
        self.assertTrue(tail.endswith("error: the last line, the real one"))
        kept = tail.splitlines()
        self.assertEqual(kept[0], lines[first - 1], "log_line names the first kept line")
        self.assertEqual(kept, lines[first - 1:])
        rec = adapters.exit_adapter(check(), log, 1, AdapterContext(root=d, scratch=d, tmp=d, results=d / "e",
                                                                     timed_out=False, seam_map=MAP))[0]
        self.assertEqual((rec.assertion, rec.log_line), (tail, first))
        self.assertEqual(summary.headline(rec), "exit 1: error: the last line, the real one")

    def test_one_giant_last_line_keeps_its_end(self) -> None:
        d = tmpdir(self)
        log = d / "out.log"
        log.write_bytes(b"a\nb\n" + b"x" * 10000 + b"END\n")
        tail, first = adapters.log_tail(log)
        self.assertTrue(tail.endswith("xEND"))
        self.assertEqual(len(tail), record.ASSERTION_MAX_BYTES)
        self.assertEqual(first, 3)

    def test_big_log_line_number(self) -> None:
        d = tmpdir(self)
        log = d / "out.log"
        with log.open("w") as fh:
            for i in range(1, 200001):
                fh.write(f"{i}\n")
        tail, first = adapters.log_tail(log)
        self.assertEqual(tail.splitlines()[0], str(first))
        self.assertEqual(tail.splitlines()[-1], "200000")

    def test_headline_prefers_the_last_error_like_line(self) -> None:
        rec = Record("x", "FAIL", reason="exit 1", assertion="error: early\ninfo: x\nFAIL: the real one\ncleanup done")
        self.assertEqual(summary.headline(rec), "exit 1: FAIL: the real one")
        self.assertEqual(summary.headline(Record("x", "FAIL", reason="exit 1", assertion="a\nb\n\n")), "exit 1: b")
        self.assertEqual(summary.headline(Record("x", "FAIL", assertion="assertion failed: left == right\n  left: 1")),
                         "assertion failed: left == right")


class F6NothingCrashesTheLastLine(unittest.TestCase):
    def test_undecodable_file_name_in_the_tree(self) -> None:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.badname", ["sh", "fx/badname.sh"]))
        done = subprocess.run(box.argv("--check", "fx.badname"), cwd=box.base, env=box.env(), capture_output=True,
                              timeout=60, start_new_session=True)
        out, err = done.stdout.decode("utf-8", "replace"), done.stderr.decode("utf-8", "replace")
        self.assertEqual(done.returncode, 1, out + err)
        self.assertNotIn("Traceback", err)
        self.assertEqual(last_line(out), "VERIFY RED (1 FAIL)")
        rec = by_id(box.results())["hygiene/fx.badname/tree"]
        self.assertIn("bad-\\xff.txt", rec["reason"])

    def test_a_writer_that_raises_still_ends_red(self) -> None:
        box = Sandbox(self)
        real = record.ResultsWriter.write

        def broken(self: record.ResultsWriter, obj: dict) -> None:
            if obj.get("kind") == "check":
                raise RuntimeError("disk on fire")
            real(self, obj)

        out, err = io.StringIO(), io.StringIO()
        cfg = engine.Config(root=box.tree, scratch=box.scratch, seam_map=MAP,
                            plan=select(MAP, Selection(checks=("fx.pass",))), lock_dir=box.locks, out=out, err=err,
                            ci=True)
        with mock.patch.object(record.ResultsWriter, "write", broken):
            result = engine.execute(cfg)
        self.assertEqual(result.exit_code, 1)
        self.assertEqual(last_line(out.getvalue()), "VERIFY RED (2 FAIL)")
        self.assertIn("runner error: RuntimeError: disk on fire", out.getvalue())
        self.assertIn("Traceback", err.getvalue(), "the runner bug's traceback goes to stderr")

    def test_writer_never_raises_on_text(self) -> None:
        d = tmpdir(self)
        writer = record.ResultsWriter(d / "r.jsonl")
        writer.write({"reason": "bad-\udcff.txt", "odd": {1, 2}})
        writer.close()
        writer.write({"after": "close"})
        self.assertIn("bad-\\udcff.txt", (d / "r.jsonl").read_text())
        self.assertEqual(record.safe_text("a\udcffb"), "a\\udcffb")
        self.assertEqual(hygiene.display_path(b"bad-\xff.txt"), "bad-\\xff.txt")


class F8OneRecordWhenItCannotStart(unittest.TestCase):
    def test_cannot_start(self) -> None:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.nostart", ["/nonexistent/t111-binary"]))
        done = box.run("--check", "fx.nostart")
        self.assertEqual(done.returncode, 1)
        rows = [r for r in box.results() if r.get("id") == "fx.nostart"]
        self.assertEqual(len(rows), 1, rows)
        self.assertTrue(rows[0]["reason"].startswith("could not start: "), rows[0]["reason"])

    def test_timeout_keeps_one_record(self) -> None:
        d = tmpdir(self)

        def parse(chk: Check, log: Path, status: int, ctx: AdapterContext) -> list[Record]:
            return [Record(chk.id, "FAIL", reason="exit -9", assertion="the adapter's words"), Record("u.t/x", "PASS")]

        recs = bare_engine(d).parse(check(timeout_s=5), parse, ran(d, Outcome(-9, True, True, 6)))
        self.assertEqual([r.id for r in recs], ["u.t", "u.t/x"])
        self.assertTrue(recs[0].reason.startswith("timeout after 5 s"))  # type: ignore[union-attr]
        self.assertEqual(recs[0].assertion, "the adapter's words")


class F9Attribution(unittest.TestCase):
    def test_strict_names_the_window(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.dirty")
        self.assertEqual(done.returncode, 1)
        rec = by_id(box.results())["hygiene/fx.dirty/tree"]
        self.assertEqual(rec["reason"], "the tree changed during the check: fx-dirty.txt (checks running in that "
                                        "window: fx.dirty)")

    def test_shared_box_skips_a_tree_change(self) -> None:
        for how in ("flag", "env"):
            with self.subTest(how=how):
                box = Sandbox(self)
                args = ["--check", "fx.dirty"]
                if how == "flag":
                    args.append("--shared-box")
                else:
                    box.extra_env["T111_SHARED_BOX"] = "1"
                done = box.run(*args)
                self.assertEqual(done.returncode, 0, done.stdout)
                self.assertEqual(last_line(done.stdout), "VERIFY GREEN (1 SKIPPED — not a full run)")
                rec = by_id(box.results())["hygiene/fx.dirty/tree"]
                self.assertEqual(rec["status"], "SKIPPED")
                self.assertTrue(rec["reason"].startswith("not attributed (shared box): the tree changed during the "
                                                         "check: fx-dirty.txt"), rec["reason"])
                self.assertTrue(box.results()[0]["shared_box"])
        req = Sandbox(self).run("--check", "fx.dirty", "--shared-box", "--require")
        self.assertEqual(req.returncode, 1, "--require turns it into a FAIL")

    def test_proven_leftover_fails_in_both_modes(self) -> None:
        box = Sandbox(self)
        done = box.run("--check", "fx.leak", "--shared-box")
        self.assertEqual(done.returncode, 1)
        self.assertEqual(by_id(box.results())["hygiene/fx.leak"]["status"], "FAIL")

    def test_look_alike_process(self) -> None:
        for shared in (False, True):
            with self.subTest(shared=shared):
                box = Sandbox(self)
                go = box.base / "go"
                box.extra_env["T111_GO"] = str(go)
                box.add_checks(unit_toml("fx.gated", ["sh", "fx/gate.sh"]))
                args = ["--check", "fx.gated"] + (["--shared-box"] if shared else [])
                runner = subprocess.Popen(box.argv(*args), cwd=box.base, env=box.env(), text=True,
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
                self.addCleanup(lambda r=runner: r.poll() is None and r.kill())
                wait_for(lambda: marked("fx.gated"), "the gated check to start")
                env = {k: v for k, v in box.env().items() if not k.startswith("T111_")}
                alike = subprocess.Popen(["111", "-I", "-c", "import time; time.sleep(60)"], executable=sys.executable,
                                         env=env, start_new_session=True, stdout=subprocess.DEVNULL,
                                         stderr=subprocess.DEVNULL)
                self.addCleanup(alike.wait, 10)
                self.addCleanup(alike.kill)
                wait_for(lambda: (hygiene.read_proc(alike.pid) or hygiene.Proc(0, 0, 0, 0, "", 0, [])).argv[:1]
                         == ["111"], "the look-alike to exec")
                go.write_text("go")
                out, err = runner.communicate(timeout=60)
                recs = by_id(box.results())
                if shared:
                    self.assertEqual(runner.returncode, 0, out + err)
                    rec = recs["hygiene/fx.gated/unattributed"]
                    self.assertEqual(rec["status"], "SKIPPED")
                    self.assertIn(f"pid {alike.pid} (111 ", rec["reason"])
                    self.assertTrue(rec["reason"].startswith("not attributed (shared box): "))
                else:
                    self.assertEqual(runner.returncode, 1, out + err)
                    rec = recs["hygiene/fx.gated"]
                    self.assertEqual(rec["status"], "FAIL")
                    self.assertIn(f"pid {alike.pid} (111 ", rec["reason"])
                    self.assertIn("[not killed: our binary, but nothing ties it to this run]", rec["reason"])
                self.assertIsNone(alike.poll(), "a look-alike is never killed")


class F10RequiresThroughCoveredBy(unittest.TestCase):
    def box(self) -> Sandbox:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.g2", ["sh", "fx/fail.sh"], kind="gate")
                       + unit_toml("fx.u2", ["sh", "fx/pass.sh"], covered_by="fx.g2")
                       + unit_toml("fx.r2", ["sh", "fx/pass.sh"], requires=["fx.u2"]))
        return box

    def test_plan_puts_the_requirement_on_the_gate(self) -> None:
        box = self.box()
        seam_map = load_map(box.map_dir)
        plan = select(seam_map, Selection(checks=("fx.r2", "fx.g2")))
        self.assertEqual([c.id for c in plan.checks], ["fx.g2", "fx.r2"])
        self.assertEqual(plan.via, {"fx.u2": "fx.g2"})
        self.assertIn("fx.u2: covered by fx.g2, which is in the plan; fx.r2's requirement is on fx.g2", plan.notes)
        alone = select(seam_map, Selection(checks=("fx.r2",)))
        self.assertEqual([c.id for c in alone.checks], ["fx.u2", "fx.r2"], "no gate in the plan: the unit is pulled")
        self.assertEqual(alone.via, {})

    def test_the_gate_status_decides(self) -> None:
        box = self.box()
        done = box.run("--check", "fx.r2,fx.g2")
        self.assertEqual(done.returncode, 1)
        rows = box.results()
        self.assertEqual(rows[0]["checks"], ["fx.g2", "fx.r2"])
        self.assertEqual(rows[0]["via"], {"fx.u2": "fx.g2"})
        rec = by_id(rows)["fx.r2"]
        self.assertEqual((rec["status"], rec["reason"]), ("SKIPPED", "requires fx.u2 (via fx.g2: FAIL)"))


class F11LogCap(unittest.TestCase):
    def test_cut_and_not_blocked(self) -> None:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.big", ["sh", "fx/big.sh"]))
        done = box.run("--check", "fx.big", "--log-cap", "1", "--keep")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        rec = by_id(box.results())["fx.big"]
        self.assertEqual((rec["status"], rec["reason"]), ("PASS", "log cut at 1 MB"))
        log = Path(rec["log"])
        self.assertLessEqual(log.stat().st_size, 1024 * 1024 + 200)
        self.assertEqual(last_line(log.read_text()), "t111: log cut at 1 MB (the rest of the output was read and "
                                                     "dropped)")
        self.assertEqual(box.results()[0]["log_cap_mb"], 1.0)

    def test_bad_values_are_usage_errors(self) -> None:
        box = Sandbox(self)
        for flag in ("--log-cap", "--lock-wait"):
            for bad in ("0", "-1", "x"):
                self.assertEqual(box.run("--check", "fx.pass", flag, bad).returncode, 2, (flag, bad))


class F12PasteableRerun(unittest.TestCase):
    def test_tokens_env_and_quoting(self) -> None:
        c = check(id="u.r", cmd=("sh", "-c", 'printf %s "$X" > "$1/out"', "_", "{scratch}"), env={"X": "{root}/v"})
        fresh = record.rerun_command(c, RerunPaths(Path("/r o"), Path("/run")))
        self.assertEqual(fresh, record.RERUN_FRESH + "X='/r o'/v sh -c 'printf %s \"$X\" > \"$1/out\"' _ "
                                                    '"$T111_RERUN"')
        kept = record.rerun_command(c, RerunPaths(Path("/r"), Path("/run"), Path("/k/u.r")))
        self.assertEqual(kept, "X=/r/v sh -c 'printf %s \"$X\" > \"$1/out\"' _ /k/u.r")
        templ = check(id="u.s", rerun="cargo test --manifest-path {root}/C.toml -- --exact {test}  # {check}")
        self.assertEqual(record.rerun_command(templ, RerunPaths(Path("/a b"), Path("/run")),
                                              {"test": "m::t one", "check": "u.s", "id": "u.s/x"}),
                         "cargo test --manifest-path '/a b'/C.toml -- --exact 'm::t one'  # u.s")
        self.assertEqual(record.default_rerun(c), "sh -c 'printf %s \"$X\" > \"$1/out\"' _ '{scratch}'",
                         "--explain's form is unchanged")

    def test_the_rendered_rerun_runs(self) -> None:
        d = tmpdir(self)
        root = d / "the root"
        root.mkdir()
        c = check(id="u.r", cmd=("sh", "-c", 'printf %s "$X" > "$1/out"', "_", "{scratch}"), env={"X": "{root}/v"})
        text = record.rerun_command(c, RerunPaths(root, d / "run"))
        tmp = d / "tmp"
        tmp.mkdir()
        done = subprocess.run(["sh", "-c", text], cwd=root, env=dict(os.environ, TMPDIR=str(tmp)), timeout=30,
                              capture_output=True, text=True, start_new_session=True)
        self.assertEqual(done.returncode, 0, done.stderr)
        made = list(tmp.glob("t111-rerun.*/out"))
        self.assertEqual(len(made), 1, "a fresh dir under $TMPDIR")
        self.assertEqual(made[0].read_text(), f"{root}/v")

    def test_records_carry_it(self) -> None:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.tok", ["sh", "-c", "test -d \"$1\" && exit 4", "_", "{tmp}"],
                                 env={"Y": "{root}"}))
        done = box.run("--check", "fx.tok")
        self.assertEqual(done.returncode, 1)
        rec = by_id(box.results())["fx.tok"]
        self.assertEqual(rec["rerun"], record.RERUN_FRESH + f"Y={shlex.quote(str(box.tree))} sh -c "
                                       "'test -d \"$1\" && exit 4' _ \"$T111_RERUN/tmp\"")


class StopsInProcess(unittest.TestCase):
    def test_no_stray_threads_or_handlers(self) -> None:
        """An in-process run restores the signal handlers it took."""
        box = Sandbox(self)
        before = {s: signal.getsignal(s) for s in engine.STOP_SIGNALS}
        out = io.StringIO()
        cfg = engine.Config(root=box.tree, scratch=box.scratch, seam_map=MAP,
                            plan=select(MAP, Selection(checks=("fx.pass",))), lock_dir=box.locks, out=out,
                            err=io.StringIO(), ci=True)
        self.assertEqual(engine.execute(cfg).exit_code, 0, out.getvalue())
        self.assertEqual({s: signal.getsignal(s) for s in engine.STOP_SIGNALS}, before)
        self.assertEqual([t.name for t in threading.enumerate() if t.name.startswith("t111-")], [])
        self.assertEqual(json.loads((box.run_dirs()[-1] / "results.jsonl").read_text().splitlines()[1])["status"],
                         "PASS")


if __name__ == "__main__":
    unittest.main()
