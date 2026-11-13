"""The X2b re-review's findings (C1–C12 of the X2c rulings), each pinned by a test that fails without its fix.

C4 (the log cap keeps the last MB), C5 (non-finite values) and C7 (the dead-run sweep asserted for its own run only)
are pinned in ``test_robustness.py`` (``F11LogCap``, ``F2StopSignals``); C6's ``find_leftovers`` half in
``test_units.py``; C1's end-to-end half in ``test_e2e.py`` (``test_known_never_hides_an_exit_status``). Waits poll a
condition with a bound; no pass or fail decision reads a duration.
"""

from __future__ import annotations

import io
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path
from unittest import mock

from t111 import adapters, cli, engine, hygiene, locks, record
from t111.adapters import AdapterContext
from t111.record import Record, RerunPaths
from t111.schema import Check, Known, SchemaError, load_map, parse_map
from t111.select import Selection, select

from .helpers import FIXTURE_MAP, Sandbox, by_id, unit_toml

MAP = load_map(FIXTURE_MAP)

DEFAULT_SIGNALS = ("import os, signal, sys\n"
                   "for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):\n"
                   "    signal.signal(s, signal.SIG_DFL)\n"
                   "os.execv(sys.argv[1], sys.argv[1:])\n")


def tmpdir(test: unittest.TestCase) -> Path:
    path = Path(tempfile.mkdtemp(prefix="t111-x2c-"))
    test.addCleanup(shutil.rmtree, path, True)
    return path


def wait_for(cond, what: str, limit_s: float = 30.0) -> None:
    deadline = time.monotonic() + limit_s
    while not cond():
        if time.monotonic() > deadline:
            raise AssertionError(f"timed out waiting for {what}")
        time.sleep(0.05)


OBSERVER = "import time; time.sleep(60)"
"""An observer's program: it stays alive and only names a path in its command line (like ``tail -f <log>``)."""


def observer_argv(path: Path) -> list[str]:
    """A process that names ``path`` in its command line, carries no run marker and sleeps (``sleep`` itself would
    refuse a path argument and exit at once)."""
    return [sys.executable, "-I", "-c", OBSERVER, str(path)]


def check(**kw: object) -> Check:
    base: dict = dict(id="u.t", kind="unit", cmd=("sh", "-c", "true"), tier="fast", layers=(1,), strength="I",
                      timeout_s=10)
    base.update(kw)
    return Check(**base)


def libtest_sh(failed_count: int, fail_lines: int = 1) -> str:
    """A ``sh -c`` program printing a cargo test binary's output (exit 101): ``fail_lines`` parsed FAILs, while the
    ``test result`` line counts ``failed_count``."""
    lines = ["running 2 tests", "test passes ... ok"]
    lines += [f"test known_fails_{i} ... FAILED" for i in range(fail_lines)]
    lines += ["", f"test result: FAILED. 1 passed; {failed_count} failed; 0 ignored; 0 measured; 0 filtered out; "
              "finished in 0.00s"]
    return "printf '%s\\n' " + " ".join("'" + ln + "'" for ln in lines) + "; exit 101"


class C1KnownNeverHidesTheExit(unittest.TestCase):
    def test_exit_adapter_records_are_engine_made(self) -> None:
        d = tmpdir(self)
        (d / "out.log").write_text("boom\n")
        ctx = AdapterContext(root=d, scratch=d, tmp=d, results=d / "e", timed_out=False, seam_map=MAP)
        for name, status in (("exit", 1), ("exit", 0), ("exit77", 1), ("exit77", 0)):
            rec = adapters.REGISTRY[name](check(), d / "out.log", status, ctx)[0]
            self.assertTrue(rec.engine, (name, status))
            known = [Known(test="u.t", condition="euid0", source="S")]
            self.assertEqual(record.apply_known(rec, known, {"euid0": True}).status, rec.status, "never eligible")

    def test_backstop_runs_again_after_known(self) -> None:
        """The parse does not account for the exit (2 failed counted, 1 parsed): a known glob on the one parsed FAIL
        must not leave the check green."""
        box = Sandbox(self)
        holds = "euid0" if os.geteuid() == 0 else "nonroot"
        box.add_checks(unit_toml("fx.c1", ["sh", "-c", libtest_sh(2)], adapter="libtest")
                       + f'\n[[known]]\ntest = "*/known_fails_*"\ncheck = "fx.c1"\ncondition = "{holds}"\n'
                         'source = "S:1"\n')
        done = box.run("--check", "fx.c1")
        self.assertEqual(done.returncode, 1, done.stdout)
        recs = by_id(box.results())
        self.assertEqual(recs["fx.c1/known_fails_0"]["status"], "SKIPPED")
        self.assertEqual((recs["fx.c1"]["status"], recs["fx.c1"]["reason"]),
                         ("FAIL", "exit 101 but no failing result was parsed"))

    def test_an_accounted_exit_stays_declared(self) -> None:
        box = Sandbox(self)
        holds = "euid0" if os.geteuid() == 0 else "nonroot"
        box.add_checks(unit_toml("fx.c1b", ["sh", "-c", libtest_sh(1)], adapter="libtest")
                       + f'\n[[known]]\ntest = "*/known_fails_*"\ncheck = "fx.c1b"\ncondition = "{holds}"\n'
                         'source = "S:1"\n')
        done = box.run("--check", "fx.c1b")
        self.assertEqual(done.returncode, 0, done.stdout)
        self.assertNotIn("fx.c1b", by_id(box.results()))


class C2OwnLocksAreScheduledNotWaited(unittest.TestCase):
    def test_a_long_sibling_never_times_the_waiter_out(self) -> None:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.lk1", ["sh", "-c", "sleep 2; echo ok: long"], lock="fxlk")
                       + unit_toml("fx.lk2", ["sh", "fx/pass.sh"], lock="fxlk")
                       + unit_toml("fx.lk3", ["sh", "fx/pass.sh"]))
        done = box.run("--check", "fx.lk1,fx.lk2,fx.lk3", "--jobs", "2", "--lock-wait", "1")
        self.assertEqual(done.returncode, 0, done.stdout + done.stderr)
        recs = by_id(box.results())
        for cid in ("fx.lk1", "fx.lk2", "fx.lk3"):
            self.assertEqual(recs[cid]["status"], "PASS", cid)
        self.assertNotIn("waiting for", done.stderr, "the scheduler never starts a check on its own run's lock")

    def fake_clock(self, release_after: int, holder_fd: int):
        """time.monotonic jumps 1000 s per call; time.sleep counts polls and releases the holder at ``release_after``
        (a deterministic wait: no real duration decides anything)."""
        state = {"t": 0.0, "polls": 0}

        def monotonic() -> float:
            state["t"] += 1000.0
            return state["t"]

        def sleep(_s: float) -> None:
            state["polls"] += 1
            if state["polls"] == release_after:
                os.close(holder_fd)

        return mock.patch.multiple(locks.time, monotonic=monotonic, sleep=sleep), state

    def test_own_run_holder_is_unbounded_other_run_is_bounded(self) -> None:
        d = tmpdir(self)
        holder = locks.Locks(d, run_id="RUN-same", pid=1)
        fd = holder.take_named("fxlk")
        waiter = locks.Locks(d, run_id="RUN-same", pid=2, wait_s=1)
        patch, state = self.fake_clock(5, fd)
        with patch:
            got = waiter.take_named("fxlk")
        os.close(got)
        self.assertEqual(state["polls"], 5, "waited past many bounds, until this run's holder let go")
        other = locks.Locks(d, run_id="RUN-other", pid=3)
        fd2 = other.take_named("fxlk")
        self.addCleanup(lambda: os.close(fd2))
        patch, _state = self.fake_clock(10 ** 9, os.dup(fd2))
        with patch, self.assertRaises(locks.LockTimeout) as caught:
            waiter.take_named("fxlk")
        self.assertIn("held by run RUN-other, pid 3", str(caught.exception))


class C3RerunWithoutATest(unittest.TestCase):
    def test_template_with_an_unset_token_falls_back_to_the_cmd(self) -> None:
        c = check(id="u.r", cmd=("cargo", "test", "--test", "api"), rerun="cargo test --test api -- --exact {test}")
        where = RerunPaths(Path("/r"), Path("/run"))
        self.assertEqual(record.rerun_command(c, where, {"id": "u.r", "test": None, "check": "u.r"}),
                         "cargo test --test api")
        self.assertEqual(record.rerun_command(c, where, {"id": "u.r/t", "test": "m::t", "check": "u.r"}),
                         "cargo test --test api -- --exact m::t")

    def test_an_engine_record_of_a_templated_check(self) -> None:
        c = check(id="u.r", cmd=("cargo", "test", "--test", "api"), rerun="cargo test --test api -- --exact {test}")
        rec = record.enrich(Record("u.r", "FAIL", reason="timeout after 5 s", engine=True), c, MAP,
                            where=RerunPaths(Path("/r"), Path("/run")))
        self.assertNotIn("{test}", rec.rerun or "")
        self.assertEqual(rec.rerun, "cargo test --test api")


class C5NonFinite(unittest.TestCase):
    def test_positive_rejects_non_finite(self) -> None:
        import argparse
        for bad in ("inf", "-inf", "nan", "1e999", "0", "-1"):
            with self.assertRaises(argparse.ArgumentTypeError, msg=bad):
                cli._positive(bad)
        self.assertEqual(cli._positive("2.5"), 2.5)


class C6NamingTheDirIsNoProof(unittest.TestCase):
    def test_sweep_run_reports_a_dir_namer_and_spares_it(self) -> None:
        d = tmpdir(self)
        run_dir = d / "RUN-c6"
        run_dir.mkdir()
        env = {k: v for k, v in os.environ.items() if not k.startswith("T111_")}
        observer = subprocess.Popen(observer_argv(run_dir / "x" / "out.log"), env=env, start_new_session=True,
                                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        self.addCleanup(observer.wait, 10)
        self.addCleanup(observer.kill)
        wait_for(lambda: str(run_dir) in (hygiene.read_proc(observer.pid) or hygiene.Proc(0, 0, 0, 0, "", 0, [])).cmdline,
                 "the observer to exec")
        left = hygiene.sweep_run(runner_pid=-1, run_id="RUN-c6", run_dir=run_dir)
        mine = [lo for lo in left if lo.proc.pid == observer.pid]
        self.assertEqual(len(mine), 1)
        self.assertFalse(mine[0].kill)
        self.assertIn("[not killed: it names the run dir", mine[0].describe())

    def test_a_tail_on_the_scratch_is_not_killed_and_shared_box_skips_it(self) -> None:
        for shared in (False, True):
            with self.subTest(shared=shared):
                box = Sandbox(self)
                go = box.base / "go"
                box.extra_env["T111_GO"] = str(go)
                box.add_checks(unit_toml("fx.gated", ["sh", "fx/gate.sh"]))
                runner = subprocess.Popen(box.argv("--check", "fx.gated", *(["--shared-box"] if shared else [])),
                                          cwd=box.base, env=box.env(), text=True, stdout=subprocess.PIPE,
                                          stderr=subprocess.PIPE, start_new_session=True)
                self.addCleanup(lambda r=runner: r.poll() is None and r.kill())

                def check_scratch() -> Path | None:
                    dirs = box.run_dirs()
                    return dirs[0] / "fx.gated" if dirs and (dirs[0] / "fx.gated").is_dir() else None

                wait_for(lambda: check_scratch() is not None, "the check's scratch")
                env = {k: v for k, v in box.env().items() if not k.startswith("T111_")}
                tail = subprocess.Popen(observer_argv(check_scratch() / "out.log"), env=env,
                                        start_new_session=True)
                self.addCleanup(tail.wait, 10)
                self.addCleanup(tail.kill)
                wait_for(lambda: "out.log" in (hygiene.read_proc(tail.pid) or hygiene.Proc(0, 0, 0, 0, "", 0, [])
                                               ).cmdline, "the observer to exec")
                go.write_text("go")
                out, err = runner.communicate(timeout=60)
                self.assertIsNone(tail.poll(), "an observer is never killed")
                recs = by_id(box.results())
                if shared:
                    self.assertEqual(runner.returncode, 0, out + err)
                    rec = recs["hygiene/fx.gated/unattributed"]
                    self.assertEqual(rec["status"], "SKIPPED")
                else:
                    self.assertEqual(runner.returncode, 1, out + err)
                    rec = recs["hygiene/fx.gated"]
                    self.assertEqual(rec["status"], "FAIL")
                self.assertIn(f"pid {tail.pid} ({sys.executable} -I -c {OBSERVER}", rec["reason"])
                self.assertIn("[not killed: it names the check's scratch dir", rec["reason"])


class C7SweepDomain(unittest.TestCase):
    def test_dead_runs_of_another_lock_dir_are_not_swept(self) -> None:
        d = tmpdir(self)
        env = {k: v for k, v in os.environ.items() if not k.startswith("T111_")}
        base = dict(env, T111_RUN="RUN-dead-c7", T111_CHECK="c.x", T111_RUNNER_PID="999999999")
        mine = subprocess.Popen(["sleep", "30"], env=dict(base, T111_LOCKS=str(d / "mine")), start_new_session=True)
        theirs = subprocess.Popen(["sleep", "31"], env=dict(base, T111_LOCKS=str(d / "theirs")),
                                  start_new_session=True)
        for p in (mine, theirs):
            self.addCleanup(p.wait, 10)
            self.addCleanup(p.kill)
        wait_for(lambda: all((hygiene.read_proc(p.pid) or hygiene.Proc(0, 0, 0, 0, "", 0, [], {})).environ
                             and (hygiene.read_proc(p.pid).environ or {}).get("T111_RUN") for p in (mine, theirs)),
                 "the sleepers to exec")
        found = {p.pid for procs in hygiene.sweep_dead_runs("RUN-me", lock_dir=d / "mine").values() for p in procs}
        self.assertIn(mine.pid, found)
        self.assertNotIn(theirs.pid, found, "another sandbox's (another lock dir's) dead run is not ours to sweep")


class C8ReservedIds(unittest.TestCase):
    def test_runner_and_run_are_refused(self) -> None:
        seams = {"seam": [{"id": "L1-L2:fx", "status": "built", "a": {"layer": 1, "files": []},
                           "b": {"layer": 2, "files": []}, "read_first": "a"}]}
        for cid in ("runner", "run"):
            doc = {"check": [{"id": cid, "kind": "unit", "cmd": ["true"], "tier": "fast", "layers": [1],
                              "strength": "I", "timeout_s": 5}]}
            with self.assertRaises(SchemaError) as caught:
                parse_map(seams, doc)
            self.assertIn("reserved", str(caught.exception))
        parse_map(seams, {"check": [{"id": "runner2", "kind": "unit", "cmd": ["true"], "tier": "fast",
                                     "layers": [1], "strength": "I", "timeout_s": 5}]})


class C9StoppedIsNeverPass(unittest.TestCase):
    def test_a_check_that_traps_term_and_exits_0_is_the_interrupt_fail(self) -> None:
        box = Sandbox(self)
        box.add_checks(unit_toml("fx.trap", ["sh", "-c", "trap 'exit 0' TERM; echo 'trap: started'; "
                                             "sleep 60 & wait $!"], timeout_s=120))
        argv = [sys.executable, "-I", "-c", DEFAULT_SIGNALS, *box.argv("--check", "fx.trap", "--kill-grace", "5")]
        proc = subprocess.Popen(argv, cwd=box.base, env=box.env(), text=True, stdout=subprocess.PIPE,
                                stderr=subprocess.PIPE, start_new_session=True)
        self.addCleanup(lambda: proc.poll() is None and proc.kill())

        def started() -> bool:
            dirs = box.run_dirs()
            log = dirs[0] / "fx.trap" / "out.log" if dirs else None
            return bool(log and log.is_file() and "trap: started" in log.read_text())

        wait_for(started, "the trapping check")
        proc.send_signal(signal.SIGTERM)
        out, err = proc.communicate(timeout=90)
        self.assertEqual(proc.returncode, 1, out + err)
        rec = by_id(box.results())["fx.trap"]
        self.assertEqual((rec["status"], rec["reason"]), ("FAIL", "interrupted by SIGTERM: the runner killed the check"))
        self.assertIn("1 check(s) killed", by_id(box.results())["runner"]["reason"])


class _Closed(io.StringIO):
    def write(self, _text: str) -> int:  # an agent's `| head` that went away
        raise BrokenPipeError(32, "Broken pipe")


class C10Signals(unittest.TestCase):
    def test_emit_never_raises(self) -> None:
        engine.emit(_Closed(), "x\n")
        closed = io.StringIO()
        closed.close()
        engine.emit(closed, "y\n")

    def test_progress_to_a_closed_stderr_is_no_runner_error(self) -> None:
        box = Sandbox(self)
        out = io.StringIO()
        cfg = engine.Config(root=box.tree, scratch=box.scratch, seam_map=MAP,
                            plan=select(MAP, Selection(checks=("fx.pass", "fx.after-pass"))), lock_dir=box.locks, out=out,
                            err=_Closed())
        result = engine.execute(cfg)
        self.assertEqual(result.exit_code, 0, out.getvalue())
        self.assertEqual(out.getvalue().rstrip().splitlines()[-1], "VERIFY GREEN")

    def test_a_signal_during_set_up_ends_red_before_anything_runs(self) -> None:
        box = Sandbox(self)
        real = engine.StopState.install

        def install_and_signal(self: engine.StopState) -> None:
            real(self)
            self.name = "SIGTERM"  # as if the signal arrived while the map loaded

        out = io.StringIO()
        with mock.patch.object(engine.StopState, "install", install_and_signal), \
                mock.patch.object(sys, "stdout", out):
            code = cli.main(["--root", str(box.tree), "--map", str(FIXTURE_MAP), "--scratch", str(box.scratch),
                             "--check", "fx.pass"])
        self.assertEqual(code, 1)
        self.assertEqual(out.getvalue().rstrip().splitlines()[-1], "VERIFY RED (1 FAIL)")
        self.assertIn("interrupted by SIGTERM before the run started", out.getvalue())
        self.assertEqual(box.run_dirs(), [], "nothing ran")

    def test_a_late_signal_still_makes_the_run_red(self) -> None:
        box = Sandbox(self)
        out = io.StringIO()
        stop = engine.StopState()
        cfg = engine.Config(root=box.tree, scratch=box.scratch, seam_map=MAP,
                            plan=select(MAP, Selection(checks=("fx.pass",))), lock_dir=box.locks, out=out,
                            err=io.StringIO(), ci=True, stop_state=stop)
        real = hygiene.reap_orphans

        def reap_then_signal(*args: object, **kw: object) -> None:
            real(*args, **kw)  # type: ignore[arg-type]
            stop.name = "SIGHUP"  # the signal lands after the checks, before the report

        with mock.patch.object(hygiene, "reap_orphans", reap_then_signal):
            result = engine.execute(cfg)
        self.assertEqual(result.exit_code, 1, out.getvalue())
        self.assertIn("runner", {r.id for r in result.records})
        self.assertTrue(out.getvalue().rstrip().splitlines()[-1].startswith("VERIFY RED"))

    def test_the_cli_installs_the_handlers_first(self) -> None:
        seen: list[object] = []
        real_load = cli.load_map

        def spy(*args: object, **kw: object):
            seen.append(signal.getsignal(signal.SIGTERM))
            return real_load(*args, **kw)  # type: ignore[arg-type]

        with mock.patch.object(cli, "load_map", spy), mock.patch.object(sys, "stdout", io.StringIO()):
            cli.main(["--map", str(FIXTURE_MAP), "--list", "--check", "fx.pass"])
        self.assertTrue(seen and getattr(seen[0], "__self__", None).__class__ is engine.StopState,
                        "a StopState handler was in place when the map loaded")


class C11ReadOnlyLockFile(unittest.TestCase):
    def test_a_read_only_holder_leaves_a_note_and_the_waiter_says_so(self) -> None:
        d = tmpdir(self)
        path = d / "verify.lock"
        path.write_text("run=RUN-stale pid=1 uid=1234\n")
        os.utime(path, (1, 1))  # the stale line of an earlier holder

        def read_only(p: Path) -> int:
            return os.open(p, os.O_RDONLY | os.O_CLOEXEC)

        with mock.patch.object(locks, "_open", read_only):
            fd = locks.Locks(d, run_id="RUN-other-user", pid=4242).take_named("verify")
        self.addCleanup(os.close, fd)
        self.assertEqual(path.read_text(), "run=RUN-stale pid=1 uid=1234\n", "the lock file could not be written")
        words: list[str] = []
        with self.assertRaises(locks.LockTimeout) as caught:
            locks.Locks(d, run_id="RUN-me", pid=7, wait_s=0.01).take_named("verify", on_wait=words.append)
        self.assertEqual(words, ["waiting for verify held by another user's runner (holder not recorded in the lock "
                                 "file; its note: run RUN-other-user, pid 4242)"])
        self.assertIn("another user's runner", str(caught.exception))
        self.assertNotIn("RUN-stale", str(caught.exception), "never the stale line")

    def test_no_mark_at_all(self) -> None:
        d = tmpdir(self)
        path = d / "verify.lock"
        path.write_text("")
        fd = os.open(path, os.O_RDONLY)
        self.addCleanup(os.close, fd)
        import fcntl
        fcntl.flock(fd, fcntl.LOCK_EX)
        words: list[str] = []
        with mock.patch.object(locks.os, "access", lambda *_a, **_k: False), self.assertRaises(locks.LockTimeout):
            locks.Locks(d, run_id="RUN-me", pid=7, wait_s=0.01).take_named("verify", on_wait=words.append)
        self.assertEqual(words, ["waiting for verify held by another user's runner (holder not recorded)"])


class C12LockWaitLineUnderCi(unittest.TestCase):
    def test_the_wait_line_prints_once_under_ci(self) -> None:
        import fcntl
        box = Sandbox(self)
        box.locks.mkdir(parents=True)
        fd = os.open(box.locks / "verify.lock", os.O_RDWR | os.O_CREAT, 0o666)
        self.addCleanup(os.close, fd)
        fcntl.flock(fd, fcntl.LOCK_EX)
        os.write(fd, b"run=RUN-holder pid=4242\n")
        done = box.run("--check", "fx.verify1", "--lock-wait", "1", "--ci")
        self.assertEqual(done.returncode, 1)
        self.assertEqual(done.stderr.count("waiting for verify held by run RUN-holder (pid 4242)"), 1, done.stderr)
        self.assertNotIn("fx.verify1 FAIL (", done.stderr, "progress lines stay off under --ci")

    def test_results_flag(self) -> None:
        args = cli.parser().parse_args(["--matrix", "--results", "r.jsonl"])
        self.assertEqual(args.results, Path("r.jsonl"))
        self.assertIsNone(cli.parser().parse_args(["--matrix"]).results)


if __name__ == "__main__":
    unittest.main()
