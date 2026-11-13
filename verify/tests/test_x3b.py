"""One regression test per finding of the X3 review (row X3b), each with a positive control.

The positive control of every test is that it fails on the adapters as they were before X3b (the adapter module of the
X3 commit): the finding it names is reproduced by the same log, and the assertion states what a correct adapter says.
The cases that need a captured shape live as goldens beside the X3 ones (``fixtures/stagewise/l1-tools-missing``,
``l1-summary-gate-dump``, ``l1-cargo-stops``, ``fixtures/emitter/log-fail-missing``); the assertions here name the
rule, the golden pins the exact records.
"""

from __future__ import annotations

import shutil
import tempfile
import time
import unittest
from pathlib import Path

from t111 import adapters, record
from t111.adapters import AdapterContext
from t111.record import Record
from t111.schema import Known, parse_map

from tests.test_adapters import _SEAMS, FIX, MAP, _check, run_case

HEADERS_L1 = (
    "== fmt ==\n== clippy (host) ==\n== test (host) ==\n== shellcheck ==\n"
    "== gate assertions (hermetic: musl-env.sh self-test, this script's summary + G2 asserts, cross-build UI gate + "
    "release asserts) ==\n")


def fixture(case: str) -> str:
    return (FIX / f"{case}.txt").read_text(encoding="utf-8")


def by_id(recs: list[Record]) -> dict[str, Record]:
    return {r.id: r for r in recs}


class EmitterNeverHidesALogFail(unittest.TestCase):
    """High: with ``T111_RESULTS`` lines present, a ``FAIL:`` line of the log the file lacks must stay a FAIL."""

    OK_ONLY = '{"status":"ok","text":"daemon up on 127.0.0.1:11180 within 4s","phase":"4. namespace + daemon"}\n'

    def test_a_fail_line_missing_from_the_file_is_kept(self) -> None:
        recs, ctx = run_case("emitter/instrumented", emit_text=self.OK_ONLY)
        fails = [r for r in recs if r.status == "FAIL"]
        self.assertEqual([r.assertion for r in fails], ["GET / serves index.html  [HTTP 500: boom]"])
        self.assertEqual(fails[0].phase, "5a. static UI + Host guard")
        self.assertIn("lacks", fails[0].reason or "")
        self.assertTrue(ctx.exit_declared, "the log's one FAIL line explains exit 1, and it is now a record")

    def test_a_fail_line_the_file_has_is_not_doubled(self) -> None:
        recs, _ctx = run_case("emitter/instrumented")  # the file holds both the ok and the FAIL line
        self.assertEqual([r.assertion for r in recs if r.status == "FAIL" and r.id.startswith("fx.emit/5a")],
                         ["GET / serves index.html  [HTTP 500: boom]"])

    def test_one_of_two_equal_fail_lines_in_the_file_keeps_the_other(self) -> None:
        log = "== 5a. static UI + Host guard\nFAIL: twin\nFAIL: twin\nSMOKE RED — 2 FAIL line(s) in 1.0s\n"
        emit = '{"status":"FAIL","text":"twin","phase":"5a. static UI + Host guard"}\n'
        recs, _ctx = run_case("emitter/instrumented", log_text=log, status=1, emit_text=emit)
        self.assertEqual(sorted(r.id for r in recs if r.status == "FAIL"), ["fx.emit/5a/twin", "fx.emit/5a/twin#2"])


class SkipLineOfTheOpenStage(unittest.TestCase):
    """Medium: a skip line that names the open stage makes that stage SKIPPED, never PASS."""

    def stage_log(self, name: str, body: str) -> str:
        """``l1-tools-missing`` with the body of stage ``name`` swapped for ``body`` (a PASS-able log of everything
        else, so the stage under test is the only thing that changed)."""
        lines = fixture("stagewise/l1-tools-missing").splitlines()
        start = next(i for i, ln in enumerate(lines) if ln.startswith(f"== {name}"))
        end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("== ")), len(lines) - 1)
        return "\n".join(lines[:start + 1] + body.splitlines() + lines[end:]) + "\n"

    def test_each_missing_tool_skips_its_stage(self) -> None:
        recs, _ctx = run_case("stagewise/l1-tools-missing")
        got = by_id(recs)
        for ident, reason in (("fx.gate1/ui", "pnpm not found"), ("fx.gate1/notices", "python3 not found"),
                              ("fx.gate1/build-scripts-meshmud-build.sh", "git not found"),
                              ("fx.gate1/presence-sidecar-meshtastic", "python3 not found")):
            with self.subTest(stage=ident):
                self.assertEqual(got[ident].status, "SKIPPED")
                self.assertTrue((got[ident].reason or "").startswith(reason), got[ident].reason)
        self.assertEqual(got["fx.gate1/ui"].seams, ["L4-L5:fx-ui"], "the stage keeps its seams")
        for gone in ("ui-checks", "notices-check", "build-scripts-gate", "sidecar-checks"):
            self.assertNotIn(f"fx.gate1/{gone}", got, "a skip line that names its own stage is not a second record")
        self.assertFalse([r for r in recs if r.status == "PASS" and r.id in (
            "fx.gate1/ui", "fx.gate1/notices", "fx.gate1/presence-sidecar-meshtastic")], "nothing ran, nothing passed")

    def test_the_reviewers_reproduction(self) -> None:
        log = self.stage_log("ui", "pnpm not found — UI checks SKIPPED (4-interface/ui: ...)")
        recs, ctx = run_case("stagewise/l1-tools-missing", log_text=log, status=0)
        got = by_id(recs)
        self.assertEqual((got["fx.gate1/ui"].status, got["fx.gate1/ui"].seams), ("SKIPPED", ["L4-L5:fx-ui"]))
        self.assertNotIn("fx.gate1/ui-checks", got)
        self.assertFalse(ctx.exit_declared)

    def test_another_stages_skip_line_stays_its_own_record(self) -> None:
        got = by_id(run_case("stagewise/l1-tools-missing")[0])
        self.assertEqual(got["fx.gate1/musl-checks"].status, "SKIPPED")
        self.assertEqual(got["fx.gate1/clippy-host"].status, "PASS", "clippy ran: the musl line is not its skip")


class StageHeaderInsideAStage(unittest.TestCase):
    """Medium: an ``== name ==`` line that is not the next declared stage is the open stage's content."""

    def test_the_summary_gate_dump_opens_no_stage(self) -> None:
        recs, _ctx = run_case("stagewise/l1-summary-gate-dump")
        got = by_id(recs)
        stopped = "not reached (stopped at gate assertions"
        for ident in ("notices", "notices-generator-tests-go", "build-scripts-meshmud-build.sh", "systemd-analyze",
                      "messaging-mesh111-workspace", "presence-sidecar-meshtastic", "ui"):
            with self.subTest(stage=ident):
                self.assertEqual(got[f"fx.gate1/{ident}"].status, "SKIPPED")
                self.assertTrue((got[f"fx.gate1/{ident}"].reason or "").startswith(stopped))
        self.assertFalse([r for r in recs if r.reason and "in stage ui" in r.reason], "the FAIL is not the UI's")
        fails = [r for r in recs if r.status == "FAIL"]
        self.assertTrue(fails and all((r.phase or "").startswith("gate assertions") for r in fails), fails)
        self.assertEqual([r for r in recs if r.status == "PASS" and "presence" in r.id], [])
        self.assertEqual([r.id for r in recs if r.seams and "L4-L5:fx-ui" in r.seams and r.status == "FAIL"], [],
                         "a dumped '== ui ==' line gives the gate assertions' FAIL no UI seam")

    def test_a_real_next_stage_still_opens(self) -> None:
        recs, _ctx = run_case("stagewise/l1-tools-missing")
        self.assertIn("fx.gate1/presence-sidecar-meshtastic", by_id(recs))
        self.assertIn("fx.gate1/ui", by_id(recs))

    def test_a_skipped_conditional_stage_does_not_hide_the_next(self) -> None:
        recs, _ctx = run_case("stagewise/l1-green-skips")  # no musl x86_64 / aarch64 'check' stages, no G2 stages
        self.assertEqual(by_id(recs)["fx.gate1/shellcheck"].status, "SKIPPED")
        self.assertIn("fx.gate1/notices", by_id(recs))


class HarnessVerdictsAreNotEligibleForKnown(unittest.TestCase):
    """Medium: a verdict on a whole harness is an engine record; ``[[known]]`` never turns it SKIPPED."""

    KNOWN = [Known(test="*", condition="euid0", source="S:1")]
    CASES = (
        ("okfail/smoke-silent-abort", None, "harness did not finish"),
        ("okfail/skipped-after-fail", None, "SKIPPED after"),
        ("okfail/red-without-fail-lines", None, "printed no FAIL: line"),
        ("okfail/aborted-no-closing", None, "harness did not finish"),
        ("pytest/empty-exit0", None, "no test summary"),
        ("table/no-table", None, "printed no result table"),
        ("buildpy/crashed", None, "build.py did not finish"),
        ("libtest/crashed-mid-run", None, "did not finish"),
        ("libtest/compile-error", None, "could not compile"),
        ("emitter/instrumented", "not json\n", "not a result object"),
    )

    def test_a_known_entry_does_not_match_a_verdict(self) -> None:
        for case, emit, words in self.CASES:
            with self.subTest(case=case):
                recs, _ctx = run_case(case, emit_text=emit)
                verdicts = [r for r in recs if words in (r.reason or "")]
                self.assertTrue(verdicts, f"{case}: no record says {words!r}")
                for rec in verdicts:
                    self.assertTrue(rec.engine, rec.reason)
                    again = record.apply_known(rec, self.KNOWN, {"euid0": True})
                    self.assertEqual(again.status, "FAIL", "known turned a harness verdict SKIPPED")

    def test_a_parsed_test_stays_eligible(self) -> None:
        recs, _ctx = run_case("libtest/red-multiline-panic")
        rec = by_id(recs)["fx.lt/tests::multi_line_panic"]
        self.assertFalse(rec.engine)
        self.assertEqual(record.apply_known(rec, self.KNOWN, {"euid0": True}).status, "SKIPPED")

    def test_the_reviewers_reproduction_stays_red(self) -> None:
        recs, _ctx = run_case("okfail/smoke-silent-abort")
        known = [Known(test="fx.smoke*", condition="euid0", source="S:1")]
        done = [record.apply_known(r, known, {"euid0": True}) for r in recs]
        self.assertTrue([r for r in done if r.status == "FAIL"], "a silent abort must stay red as root")


class CargoStopsAtTheFirstFailingBinary(unittest.TestCase):
    """Medium: units whose binary cargo never reached are SKIPPED ``not reached``, not absent."""

    def test_units_after_the_failing_binary_are_not_reached(self) -> None:
        recs, ctx = run_case("stagewise/l1-cargo-stops")
        got = by_id(recs)
        for unit in ("fx.node.lib", "fx.node.lib-relay"):
            with self.subTest(unit=unit):
                self.assertEqual(got[unit].status, "SKIPPED")
                self.assertEqual(got[unit].reason, "not reached (cargo stopped at `-p node111 --test api_v1_relay`)")
                self.assertEqual(got[unit].check, unit)
                self.assertEqual(got[unit].phase, "test (host)")
        self.assertNotIn("fx.node.relay", got, "the unit whose binary ran has its tests, not a not-reached record")
        self.assertEqual(got["fx.node.relay/relay_maps_a_problem_code"].status, "FAIL")
        self.assertNotIn("fx.mesh.lib", got, "a unit another gate covers is not this stage's")
        self.assertTrue(ctx.exit_declared)

    def test_no_fail_fast_ran_every_binary(self) -> None:
        log = fixture("stagewise/l1-cargo-stops") + "\nerror: 1 target failed:\n    `-p node111 --test api_v1_relay`\n"
        recs, _ctx = run_case("stagewise/l1-cargo-stops", log_text=log, status=101)
        self.assertEqual([r.id for r in recs if (r.reason or "").startswith("not reached (cargo")], [])

    def test_a_green_run_has_no_not_reached(self) -> None:
        recs, _ctx = run_case("stagewise/l1-green-skips")
        self.assertEqual([r.id for r in recs if (r.reason or "").startswith("not reached (cargo")], [])


class SkipLineQuotedByAFailLine(unittest.TestCase):
    """Low: a ``FAIL:`` line that quotes a skip line is a FAIL, not a SKIPPED record."""

    def test_the_fail_line_is_a_result(self) -> None:
        log = HEADERS_L1 + (
            "check-gate: FAIL: G2 skipped (rc=0, last line: !! musl cross toolchains absent — G2 (SQLCipher static "
            "link) SKIPPED; REQUIRE_G2=1 makes this a failure)\ncheck-gate: 1 case(s) failed\n")
        recs, _ctx = run_case("stagewise/l1-summary-gate-dump", log_text=log, status=1)
        fails = [r for r in recs if r.status == "FAIL"]
        self.assertEqual([r.assertion for r in fails][:1], [
            "G2 skipped (rc=0, last line: !! musl cross toolchains absent — G2 (SQLCipher static link) SKIPPED; "
            "REQUIRE_G2=1 makes this a failure)"])
        self.assertNotIn("fx.gate1/g2-sqlcipher-static-link", by_id(recs), "no bogus SKIPPED for the quoted line")

    def test_a_real_skip_line_is_still_a_skip(self) -> None:
        got = by_id(run_case("stagewise/l1-tools-missing")[0])
        self.assertEqual(got["fx.gate1/g2-sqlcipher-static-link"].status, "SKIPPED")


class OkFailIdsAreStable(unittest.TestCase):
    """Low: a description's id piece does not change from run to run (hashed asset names, random tokens)."""

    def test_a_hashed_asset_name_slugs_alike(self) -> None:
        a = adapters.desc_slug("GET /assets/index-B8fJ2kLq.js → 200 text/javascript, cache immutable")
        b = adapters.desc_slug("GET /assets/index-Cx81_aZQ.js → 200 text/javascript, cache immutable")
        self.assertEqual(a, b)
        self.assertEqual(a, "get-assets-index-n.js-200-text-javascript-cache-immutable")

    def test_a_random_id_slugs_alike(self) -> None:
        self.assertEqual(adapters.desc_slug("proposal b3k9x2m7 decided"), adapters.desc_slug("proposal q8w4e1r6 decided"))

    def test_real_words_with_digits_stay(self) -> None:
        self.assertEqual(adapters.slug("sha256 matches"), "sha256-matches")
        self.assertEqual(adapters.slug("mesh111 node111 ed25519 aes256gcm"), "mesh111-node111-ed25519-aes256gcm")
        self.assertEqual(adapters.slug("x86_64 musl"), "x86-64-musl")
        self.assertEqual(adapters.slug("GET /static/readme-heading.png"), "get-static-readme-heading.png")

    def test_the_okfail_record_id_is_the_same_across_runs(self) -> None:
        def ident(asset: str) -> str:
            log = (f"== 5a. static UI + Host guard\nok: GET /assets/{asset} → 200 text/javascript\n"
                   "SMOKE GREEN — all passed in 1.0s\n")
            recs, _ctx = run_case("emitter/not-instrumented", log_text=log, status=0, emit_text="")
            return next(r.id for r in recs if r.id != "fx.emit")
        self.assertEqual(ident("index-B8fJ2kLq.js"), ident("index-Cx81_aZQ.js"))


class StageIdsDoNotCollide(unittest.TestCase):
    """Low: a stage id is the same whatever else a run printed (no ``#2`` by meeting order)."""

    def run_gate(self, script: str, log: str, status: int) -> list[Record]:
        root = Path(tempfile.mkdtemp(prefix="t111-x3b-"))
        self.addCleanup(shutil.rmtree, root, True)
        (root / "gx").mkdir()
        (root / "gx" / "check.sh").write_text(script, encoding="utf-8")
        seam_map = parse_map(_SEAMS, {"check": [_check("fx.gx", "gate", ["sh", "gx/check.sh"], "stagewise")]})
        (root / "out.log").write_text(log, encoding="utf-8")
        scratch = root / "scratch"
        (scratch / "tmp").mkdir(parents=True)
        ctx = AdapterContext(root=root, scratch=scratch, tmp=scratch / "tmp", results=scratch / "emit.jsonl",
                             timed_out=False, seam_map=seam_map)
        check = seam_map.check("fx.gx")
        assert check is not None
        return adapters.stagewise_adapter(check, root / "out.log", status, ctx)

    def test_a_cut_that_falls_between_two_words_keeps_both(self) -> None:
        self.assertEqual(adapters.stage_slug("G2: SQLCipher static link (mesh111 tests, musl aarch64)"),
                         "g2-sqlcipher-static-link-mesh111")
        self.assertEqual(adapters.slug("abc def ghi", 7), "abc-def")
        self.assertEqual(adapters.slug("abc def ghi", 9), "abc-def")
        self.assertEqual(adapters.slug("abcdefghij", 4), "abcd")

    def test_the_g2_stages_and_their_skip_line_have_three_ids(self) -> None:
        recs, _ctx = run_case("stagewise/l1-clippy-fail")
        ids = [r.id for r in recs if r.id.startswith("fx.gate1/g2-")]
        self.assertEqual(len(ids), 2)
        self.assertEqual(len(set(ids)), 2)
        self.assertFalse([i for i in ids if "#" in i], ids)

    def test_stages_that_share_32_characters_get_a_hash_each(self) -> None:
        script = ('echo "== stage alpha with a long common prefix, number one =="\ntrue\n'
                  'echo "== stage alpha with a long common prefix, number two =="\ntrue\n')
        log = "== stage alpha with a long common prefix, number one ==\nboom\n"
        recs = self.run_gate(script, log, 1)
        ids = [r.id for r in recs]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertFalse([i for i in ids if "#" in i], ids)
        self.assertTrue(all(len(i.rsplit("-", 1)[-1]) == 6 for i in ids), ids)
        # the same ids whatever else the run printed: a skip line before the stop changes nothing
        again = self.run_gate(script, "!! tool absent — stage alpha with a long common prefix SKIPPED\n" + log, 1)
        self.assertEqual({r.id for r in recs}, {r.id for r in again if r.reason and r.reason.startswith("not reached")
                                                or r.status == "FAIL"})

    def test_attributed_records_say_which_stage_ran_them(self) -> None:
        """The ids of gate-attributed records are the unit's own on purpose (one test, one id, whoever ran it);
        ``phase`` carries the stage of the run, and the engine writes the check's log next to it."""
        got = by_id(run_case("stagewise/l1-green-skips")[0])
        self.assertEqual(got["fx.mesh.contacts/node_id_contacts_survive_restarts_until_forgotten"].phase,
                         "messaging (mesh111 workspace)/test")
        self.assertEqual(got["fx.node.relay/relay_streams_until_done"].phase, "test (host)")


class StageSkipLineIsLinear(unittest.TestCase):
    """Low: a long whitespace run costs the stage-skip rule one scan, not a quadratic backtrack."""

    def test_a_padded_line_is_cheap(self) -> None:
        log = "== fmt ==\n" + " " * 60000 + "x\n" + "ok: after\n"
        started = time.monotonic()
        recs, _ctx = run_case("stagewise/l1-green-skips", log_text=log, status=0)
        took = time.monotonic() - started
        self.assertTrue(recs)
        self.assertLess(took, 0.5, "the stage-skip rule backtracks on whitespace")  # a record of speed, not a verdict

    def test_the_skip_shapes(self) -> None:
        for text, named in (("shellcheck not found — SKIPPED", ""),
                            ("pnpm not found — UI checks SKIPPED (4-interface/ui: pnpm check)", "UI checks"),
                            ("!! musl cross toolchains absent (~/x) — musl checks SKIPPED", "musl checks"),
                            ("!! musl absent — G2 (SQLCipher static link) SKIPPED; REQUIRE_G2=1", "G2 (SQLCipher static link)"),
                            ("a — b — c SKIPPED", "c")):
            with self.subTest(text=text):
                self.assertEqual(adapters.stage_skip(text), named)
        for text in ("no dash SKIPPED", "a — b skipped", "ok: fine", "SKIPPED — nothing after"):
            with self.subTest(text=text):
                self.assertIsNone(adapters.stage_skip(text))


if __name__ == "__main__":
    unittest.main()
