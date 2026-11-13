"""The adapters of options.md §3.4 against golden logs (row X3).

Every case is ``fixtures/<adapter>/<case>.txt`` (a captured log of a real run, paths made neutral, or a log built from
the scripts' own ``printf``/``echo`` lines and the real cargo / vitest / unittest formats) with the expected records
beside it as ``<case>.jsonl``: the first line ``{"exit_declared": …}``, then one projected record per line (the fields
an adapter sets, after the engine's cascade link ``after``). ``<case>.vitest.json`` is the JSON reporter's file of a
vitest unit, ``<case>.emit.jsonl`` a ``T111_RESULTS`` file. ``CASES`` names each case's check (from the fixture map
below) and exit status.

Regenerate after a deliberate change with ``T111_REGEN=1`` and read the diff: a golden is only as good as its review.

The positive control (``PlantedFailure``): every registered adapter has a plant, a FAIL planted into one of its green
logs, and the test fails when the adapter returns no FAIL for it.
"""

from __future__ import annotations

import json
import os
import shutil
import tempfile
import unittest
from pathlib import Path
from typing import Any, Callable

from t111 import adapters, record
from t111.adapters import AdapterContext
from t111.record import Record
from t111.schema import SeamMap, parse_map

FIX = Path(__file__).resolve().parent / "fixtures"

_SEAMS = {"seam": [
    {"id": sid, "status": "built", "spec": [], "a": {"layer": a, "files": [f"fx/{sid}-a"]},
     "b": {"layer": b, "files": [f"fx/{sid}-b"]}, "read_first": "a"}
    for sid, a, b in (("L1-L2:fx-alpha", 1, 2), ("L4-L5:fx-ui", 4, 5), ("L5-L6:fx-mesh", 5, 6),
                      ("L3-L4:fx-agent", 3, 4), ("L1-X:fx-gate", 1, "X"))]}


def _check(cid: str, kind: str, cmd: list[str], adapter: str, **kw: Any) -> dict[str, Any]:
    base: dict[str, Any] = {"id": cid, "kind": kind, "cmd": cmd, "tier": "fast", "layers": [1], "strength": "I",
                            "timeout_s": 60, "adapter": adapter, "seams": ["L1-L2:fx-alpha"]}
    base.update(kw)
    return base


def _cargo(cid: str, *args: str, covered_by: str = "", seams: tuple[str, ...] = ("L5-L6:fx-mesh",)) -> dict:
    return _check(cid, "unit", ["cargo", "test", "--manifest-path", "x/Cargo.toml", *args], "libtest",
                  covered_by=covered_by, seams=list(seams), rerun=f"cargo test {' '.join(args)} -- --exact {{test}}")


_CHECKS = {"check": [
    _cargo("fx.lt", "-p", "redcrate", seams=("L1-L2:fx-alpha",)),
    _cargo("fx.id111.lib", "-p", "id111", "--lib", seams=("L1-L2:fx-alpha",)),
    _check("fx.gate5", "gate", ["sh", "stagewise/5-messaging/scripts/check.sh.txt"], "stagewise"),
    _cargo("fx.mesh.lib", "-p", "mesh111", "--lib", covered_by="fx.gate5"),
    _cargo("fx.mesh.forward", "-p", "mesh111", "--test", "forward", covered_by="fx.gate5"),
    _cargo("fx.mesh.contacts", "-p", "mesh111", "--test", "contacts", covered_by="fx.gate5"),
    _check("fx.gate1", "gate", ["sh", "stagewise/1-bootstrap/scripts/check.sh.txt"], "stagewise",
           phases={"clippy": ["L1-X:fx-gate"], "ui": ["L4-L5:fx-ui"]}),
    _cargo("fx.node.lib", "-p", "node111", "--lib", covered_by="fx.gate1", seams=("L1-X:fx-gate",)),
    _cargo("fx.node.lib-relay", "-p", "node111", "--lib", "relay::", covered_by="fx.gate1",
           seams=("L3-L4:fx-agent",)),
    _cargo("fx.node.relay", "-p", "node111", "--test", "api_v1_relay", covered_by="fx.gate1",
           seams=("L3-L4:fx-agent",)),
    _check("fx.presence", "unit", ["sh", "5-messaging/sidecars/meshtastic-presence/run-tests.sh"], "pytest",
           covered_by="fx.gate1", seams=["L5-L6:fx-mesh"]),
    _check("fx.presence-lock", "unit", ["python3", "-I", "5-messaging/sidecars/meshtastic-presence/gen_lock.py",
                                        "--verify"], "exit", covered_by="fx.gate1"),
    _check("fx.notices.l5", "unit", ["python3", "-I", "-B", "-m", "unittest", "discover", "-s",
                                     "scripts/notices/tests"], "pytest", covered_by="fx.gate1"),
    _check("fx.notices.native", "unit", ["python3", "-I", "-B", "-m", "unittest", "discover", "-s", "scripts/tests"],
           "pytest", covered_by="fx.gate1"),
    _check("fx.gate3", "gate", ["sh", "stagewise/3-intelligence/scripts/check.sh.txt"], "stagewise"),
    _cargo("fx.agent.chat_loop", "-p", "agent111", "--test", "chat_loop", covered_by="fx.gate3",
           seams=("L3-L4:fx-agent",)),
    _check("fx.ui", "gate", ["pnpm", "check"], "vitest", cwd="4-interface/ui", seams=["L4-L5:fx-ui"]),
    _check("fx.ui.contacts", "unit", ["pnpm", "exec", "vitest", "run", "test/contacts.test.ts"], "vitest",
           cwd="4-interface/ui", covered_by="fx.ui", seams=["L4-L5:fx-ui"]),
    _check("fx.ui.http", "unit", ["pnpm", "exec", "vitest", "run", "test/http.test.ts"], "vitest",
           cwd="4-interface/ui", covered_by="fx.ui", seams=["L4-L5:fx-ui"]),
    _check("fx.vt.red", "unit", ["pnpm", "exec", "vitest", "run", "--root", "vt", "--reporter=default",
                                 "--reporter=json", "--outputFile={scratch}/vitest.json"], "vitest"),
    _check("fx.vt.gate", "gate", ["pnpm", "check"], "vitest", cwd="vt"),
    _check("fx.vt.redfile", "unit", ["pnpm", "exec", "vitest", "run", "test/red.test.ts"], "vitest", cwd="vt",
           covered_by="fx.vt.gate"),
    _check("fx.vt.greenfile", "unit", ["pnpm", "exec", "vitest", "run", "test/green.test.ts"], "vitest", cwd="vt",
           covered_by="fx.vt.gate"),
    _check("fx.py", "unit", ["sh", "x/run-tests.sh"], "pytest"),
    _check("fx.smoke", "unit", ["bash", "okfail/smoke.sh.txt"], "okfail",
           phases={"4.": ["L1-L2:fx-alpha"], "5a": ["L4-L5:fx-ui"], "5e": ["L3-L4:fx-agent"]}),
    _check("fx.ok", "unit", ["sh", "x/harness.sh"], "okfail"),
    _check("fx.emit", "unit", ["bash", "x/instrumented.sh"], "emitter",
           phases={"4.": ["L1-L2:fx-alpha"], "5a": ["L4-L5:fx-ui"]}),
    _check("fx.table", "unit", ["bash", "x/run.sh"], "table"),
    _check("fx.kb", "gate", ["python3", "x/build.py", "--check"], "buildpy"),
    _check("fx.e77", "unit", ["sh", "x/e2e.sh", "--namespace"], "exit77"),
    _check("fx.exit", "unit", ["python3", "x/build.py", "--selftest"], "exit"),
]}

MAP: SeamMap = parse_map(_SEAMS, _CHECKS)

CASES: dict[str, tuple[str, int]] = {
    # libtest (unit runs)
    "libtest/green": ("fx.lt", 0),
    "libtest/red-multiline-panic": ("fx.lt", 101),
    "libtest/crashed-mid-run": ("fx.lt", 101),
    "libtest/interleaved-crash-no-fail-fast": ("fx.lt", 101),
    "libtest/quiet": ("fx.lt", 101),
    "libtest/compile-error": ("fx.lt", 101),
    "libtest/id111-lib-green": ("fx.id111.lib", 0),
    # stagewise (gate runs: records attributed to the units they cover)
    "stagewise/l5-green": ("fx.gate5", 0),
    "stagewise/l5-red-test": ("fx.gate5", 101),
    "stagewise/l1-clippy-fail": ("fx.gate1", 101),
    "stagewise/l1-green-skips": ("fx.gate1", 0),
    "stagewise/l3-no-headers": ("fx.gate3", 101),
    "stagewise/l1-tools-missing": ("fx.gate1", 0),
    "stagewise/l1-summary-gate-dump": ("fx.gate1", 1),
    "stagewise/l1-summary-gate-dump-notices": ("fx.gate1", 1),
    "stagewise/l1-cargo-stops": ("fx.gate1", 101),
    # vitest
    "vitest/unit-json-green": ("fx.ui.contacts", 0),
    "vitest/unit-json-red": ("fx.vt.red", 1),
    "vitest/unit-text-green": ("fx.ui.contacts", 0),
    "vitest/gate-text-red": ("fx.vt.gate", 1),
    "vitest/typecheck-red": ("fx.ui", 1),
    # pytest / unittest
    "pytest/unittest-red": ("fx.py", 1),
    "pytest/unittest-red-verbose": ("fx.py", 1),
    "pytest/unittest-green-skip": ("fx.py", 0),
    "pytest/presence-sidecar-green": ("fx.py", 0),
    "pytest/pytest-red-rs": ("fx.py", 1),
    "pytest/pytest-green": ("fx.py", 0),
    "pytest/crashed-no-summary": ("fx.py", 1),
    "pytest/empty-exit0": ("fx.py", 0),
    # okfail
    "okfail/t0-stub-skipped": ("fx.ok", 0),
    "okfail/t0-stub-require-fail": ("fx.ok", 1),
    "okfail/check-summary-green": ("fx.ok", 0),
    "okfail/shim-red": ("fx.ok", 1),
    "okfail/lint-green": ("fx.ok", 0),
    "okfail/smoke-silent-abort": ("fx.smoke", 0),
    "okfail/smoke-red-kept": ("fx.smoke", 1),
    "okfail/skipped-after-fail": ("fx.ok", 0),
    "okfail/exit0-with-fail": ("fx.ok", 0),
    "okfail/red-closing": ("fx.ok", 1),
    "okfail/aborted-no-closing": ("fx.ok", 1),
    "okfail/red-without-fail-lines": ("fx.ok", 1),
    "okfail/nested-green-then-abort": ("fx.ok", 0),
    "okfail/green-no-result": ("fx.ok", 0),
    # emitter
    "emitter/instrumented": ("fx.emit", 1),
    "emitter/not-instrumented": ("fx.emit", 0),
    "emitter/log-fail-missing": ("fx.emit", 1),
    # table, buildpy
    "table/green": ("fx.table", 0),
    "table/red": ("fx.table", 1),
    "table/no-table": ("fx.table", 1),
    "buildpy/green": ("fx.kb", 0),
    "buildpy/red": ("fx.kb", 1),
    "buildpy/crashed": ("fx.kb", 1),
    # exit77, exit
    "exit77/skip": ("fx.e77", 77),
    "exit77/pass": ("fx.e77", 0),
    "exit77/fail": ("fx.e77", 1),
    "exit77/77-without-reason": ("fx.e77", 77),
    "exit/green": ("fx.exit", 0),
    "exit/red": ("fx.exit", 1),
}

FIELDS = ("id", "status", "reason", "assertion", "log_line", "phase", "seams", "test", "check", "after", "kept")
"""The record fields a golden pins (what an adapter sets, plus the cascade link)."""


def project(rec: Record) -> dict[str, Any]:
    """The golden form of a record: the pinned fields that are set."""
    out: dict[str, Any] = {}
    for name in FIELDS:
        value = getattr(rec, name)
        if value is not None:
            out[name] = value
    return out


def run_case(case: str, log_text: str | None = None, status: int | None = None,
             emit_text: str | None = None) -> tuple[list[Record], AdapterContext]:
    """Run the case's adapter over its log (or ``log_text``) in a temp scratch; return the records and the context."""
    check_id, default_status = CASES[case]
    check = MAP.check(check_id)
    assert check is not None, check_id
    scratch = Path(tempfile.mkdtemp(prefix="t111-x3-"))
    try:
        (scratch / "tmp").mkdir()
        log = FIX / f"{case}.txt"
        if log_text is not None:
            log = scratch / "out.log"
            log.write_text(log_text, encoding="utf-8")
        vitest_json = FIX / f"{case}.vitest.json"
        if vitest_json.is_file():
            shutil.copy(vitest_json, scratch / "vitest.json")
        results = scratch / "emit.jsonl"
        emit = FIX / f"{case}.emit.jsonl"
        if emit_text is not None:
            results.write_text(emit_text, encoding="utf-8")
        elif emit.is_file():
            shutil.copy(emit, results)
        ctx = AdapterContext(root=FIX, scratch=scratch, tmp=scratch / "tmp", results=results, timed_out=False,
                             seam_map=MAP)
        adapter = adapters.get(check.adapter)
        assert adapter is not None, check.adapter
        recs = adapter(check, log, default_status if status is None else status, ctx)
        record.mark_after(recs, one_harness=True)
        return recs, ctx
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def read_golden(case: str) -> tuple[dict[str, Any], list[dict[str, Any]]]:
    lines = [json.loads(line) for line in (FIX / f"{case}.jsonl").read_text(encoding="utf-8").splitlines()
             if line.strip()]
    return lines[0], lines[1:]


def write_golden(case: str, declared: bool, recs: list[Record]) -> None:
    rows = [json.dumps({"exit_declared": declared}, ensure_ascii=False)]
    rows += [json.dumps(project(r), ensure_ascii=False) for r in recs]
    (FIX / f"{case}.jsonl").write_text("\n".join(rows) + "\n", encoding="utf-8")


class Goldens(unittest.TestCase):
    def test_every_fixture_log_is_a_case(self) -> None:
        logs = {str(p.relative_to(FIX).with_suffix("")) for p in FIX.glob("*/*.txt")
                if p.parent.name not in ("map", "lint") and not p.name.endswith(".sh.txt")}
        self.assertEqual(logs, set(CASES), "a fixture log without a case, or a case without its log")

    def test_goldens(self) -> None:
        regen = os.environ.get("T111_REGEN") == "1"
        for case in sorted(CASES):
            with self.subTest(case=case):
                recs, ctx = run_case(case)
                if regen:
                    write_golden(case, ctx.exit_declared, recs)
                meta, expected = read_golden(case)
                self.assertEqual([project(r) for r in recs], expected)
                self.assertEqual(ctx.exit_declared, meta["exit_declared"], "exit_declared")
                if check_adapter(case) not in ("exit", "exit77"):  # X3b: only a harness verdict is an engine record
                    self.assertEqual([r.id for r in recs if r.engine and (r.test or not r.reason)], [],
                                     "outside the exit-status adapters an engine record is a verdict (a reason, no test)")

    def test_every_adapter_has_golden_cases(self) -> None:
        used = {check_adapter(case) for case in CASES}
        self.assertEqual(set(adapters.REGISTRY) - used - {"unittest"}, set(),
                         "an adapter without a golden case (unittest is pytest's alias)")


def check_adapter(case: str) -> str:
    check = MAP.check(CASES[case][0])
    assert check is not None
    return check.adapter


class LyingShapes(unittest.TestCase):
    """The honesty rules of options.md §3.4, each named on its golden."""

    def fails(self, case: str) -> list[Record]:
        recs, _ctx = run_case(case)
        return [r for r in recs if r.status == "FAIL"]

    def test_smoke_silent_abort_is_red(self) -> None:
        verdict = [r for r in self.fails("okfail/smoke-silent-abort") if r.id == "fx.smoke"]
        self.assertEqual([r.reason for r in verdict],
                         ["harness did not finish (last phase: 5b. /api/v1 reads (loopback))"])

    def test_skipped_after_fail_is_red(self) -> None:
        verdict = [r for r in self.fails("okfail/skipped-after-fail") if r.id == "fx.ok"]
        self.assertTrue(verdict and verdict[0].reason.startswith("SKIPPED after 1 FAIL line(s): "))

    def test_exit0_with_a_fail_line_is_red(self) -> None:
        self.assertTrue(self.fails("okfail/exit0-with-fail"))

    def test_a_nested_green_is_not_the_closing_line(self) -> None:
        verdict = [r for r in self.fails("okfail/nested-green-then-abort") if r.id == "fx.ok"]
        self.assertEqual([r.reason for r in verdict], ["harness did not finish (last phase: 4. namespace + daemon)"])

    def test_crashed_binary_is_one_fail(self) -> None:
        fails = self.fails("libtest/crashed-mid-run")
        self.assertEqual(len(fails), 1)
        self.assertEqual(fails[0].reason, "the test binary did not finish (signal: 6, SIGABRT: process abort signal)")

    def test_interleaved_crash_names_the_test(self) -> None:
        fails = {r.id: r for r in self.fails("libtest/interleaved-crash-no-fail-fast")}
        self.assertIn("fx.lt/b_crashes", fails)
        self.assertTrue(fails["fx.lt/b_crashes"].reason.startswith("the test binary died during this test"))

    def test_multi_line_panic_is_the_assertion(self) -> None:
        fails = {r.id: r for r in self.fails("libtest/red-multiline-panic")}
        text = fails["fx.lt/tests::multi_line_panic"].assertion or ""
        self.assertTrue(text.startswith("thread 'tests::multi_line_panic' (12263) panicked at src/lib.rs:13:9:"))
        self.assertTrue(text.endswith("first line of the message\nsecond line: details\nthird line"))
        self.assertNotIn("stack backtrace", text)

    def test_stage_stop_marks_the_rest_not_reached(self) -> None:
        recs, _ctx = run_case("stagewise/l1-clippy-fail")
        by_id = {r.id: r for r in recs}
        self.assertEqual(by_id["fx.gate1/clippy-host"].status, "FAIL")
        self.assertEqual(by_id["fx.gate1/test-host"].reason, "not reached (stopped at clippy (host))")
        self.assertEqual(by_id["fx.gate1/ui"].status, "SKIPPED")

    def test_gate_records_carry_the_unit(self) -> None:
        recs, _ctx = run_case("stagewise/l1-green-skips")
        by_id = {r.id: r for r in recs}
        self.assertEqual(by_id["fx.node.relay/relay_streams_until_done"].check, "fx.node.relay")
        self.assertEqual(by_id["fx.node.lib-relay/relay::netwatch::tests::a_flap_is_debounced"].check,
                         "fx.node.lib-relay", "a unit's own filter wins for the tests it matches")
        self.assertEqual(by_id["fx.node.lib/daemon::config::tests::defaults_hold"].check, "fx.node.lib")
        self.assertEqual(by_id["fx.mesh.contacts/node_id_contacts_survive_restarts_until_forgotten"].check,
                         "fx.mesh.contacts", "the nested gate's units")
        self.assertIn("fx.gate1/test-host/unmapped_bin/nobody_covers_this", by_id)
        self.assertEqual(by_id["fx.presence"].check, "fx.presence", "the one unit a stage runs")
        self.assertEqual(by_id["fx.ui.contacts"].status, "PASS")


class PlantedFailure(unittest.TestCase):
    """Positive control: a FAIL planted into a green log of every adapter must surface as a FAIL record."""

    PLANTS: dict[str, tuple[str, Callable[[str], str], int, str]] = {
        "libtest": ("libtest/green", lambda t: t.replace("test zulu_ok ... ok", "test zulu_ok ... FAILED"), 101,
                    "fx.lt/zulu_ok"),
        "stagewise": ("stagewise/l5-green", lambda t: t.replace(
            "test node_id_contacts_survive_restarts_until_forgotten ... ok",
            "test node_id_contacts_survive_restarts_until_forgotten ... FAILED"), 101,
            "fx.mesh.contacts/node_id_contacts_survive_restarts_until_forgotten"),
        "vitest": ("vitest/unit-text-green", lambda t: t.replace(
            " ✓ test/contacts.test.ts (50 tests) 252ms",
            " ❯ test/contacts.test.ts (50 tests | 1 failed) 252ms\n   × parseNodeId > planted 1ms\n     → planted"),
            1, "fx.ui.contacts/parseNodeId > planted"),
        "pytest": ("pytest/unittest-green-skip", lambda t: t.replace(
            ".s\n", "F.s\n" + "=" * 70 + "\nFAIL: test_planted (test_x.X.test_planted)\n" + "-" * 70 +
            "\nTraceback (most recent call last):\n  File \"t.py\", line 1, in test_planted\nAssertionError: planted\n"
        ).replace("OK (skipped=1)", "FAILED (failures=1, skipped=1)"), 1, "fx.py/test_x.X.test_planted"),
        "okfail": ("okfail/check-summary-green", lambda t: t.replace(
            "check-gate: ALL CASES GREEN", "check-gate: FAIL: planted\ncheck-gate: ALL CASES GREEN"), 0,
            "fx.ok/planted"),
        "emitter": ("emitter/not-instrumented", lambda t: t, 0, "fx.emit/5a/planted"),
        "table": ("table/green", lambda t: t.replace(
            "keyset-project.json                              2      12     12     0     0  PASS",
            "keyset-project.json                              2      12     11     1     0  FAIL"), 1,
            "fx.table/keyset-project.json"),
        "buildpy": ("buildpy/green", lambda t: t.replace("\n0 error(s), 5 warning(s)",
                                                         "  ERROR:4  F02: planted\n\n1 error(s), 5 warning(s)"), 1,
                    "fx.kb/recipes/install-meshtastic-on-heltec-wifi-lora-32-v3.md:4/F02"),
        "exit": ("exit/green", lambda t: t, 1, "fx.exit"),
        "exit77": ("exit77/pass", lambda t: t, 1, "fx.e77"),
    }

    def test_every_adapter_has_a_plant(self) -> None:
        self.assertEqual(set(adapters.REGISTRY) - set(self.PLANTS) - {"unittest"}, set())

    def test_planted_fail_surfaces(self) -> None:
        for name, (case, plant, status, planted_id) in sorted(self.PLANTS.items()):
            with self.subTest(adapter=name):
                green, _ctx = run_case(case)
                self.assertFalse([r for r in green if r.status == "FAIL"], "the unplanted log is green")
                text = (FIX / f"{case}.txt").read_text(encoding="utf-8")
                planted = plant(text)
                emit = None
                if name == "emitter":
                    emit = '{"status":"FAIL","text":"planted","phase":"5a. static UI"}\n'
                elif name not in ("exit", "exit77"):  # those two are planted through the exit status
                    self.assertNotEqual(planted, text, "the plant changed the log")
                recs, _ctx = run_case(case, log_text=planted, status=status, emit_text=emit)
                fails = {r.id for r in recs if r.status == "FAIL"}
                self.assertIn(planted_id, fails)


if __name__ == "__main__":
    unittest.main()
