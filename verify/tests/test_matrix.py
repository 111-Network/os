"""``--matrix``: the tables come from the map, the last statuses from a results.jsonl."""

from __future__ import annotations

import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from t111 import matrix
from t111.matrix import NOT_RUN, Results, best_strength, render
from t111.record import read_results
from t111.schema import load_map

from .helpers import RUN_PY
from .test_lint import compose_map, make_tree


def run_row(**kw) -> dict:
    row = {"v": 1, "kind": "check", "id": "x", "check": None, "seams": [], "status": "PASS"}
    row.update(kw)
    return row


RESULTS = [
    {"v": 1, "kind": "run", "run_id": "20261111T000000Z-abc1234", "head": "abc1234def5678", "dirty": False},
    run_row(id="l1.alpha", check="l1.alpha", seams=["L1-L2:alpha"], status="PASS"),
    run_row(id="l1.alpha/p/two", check="l1.alpha", seams=["L1-L2:alpha"], status="FAIL"),
    run_row(id="hygiene/l1.alpha", check=None, seams=[], status="FAIL"),
    run_row(id="l1.gate/5e/x", check="l1.gate", seams=["L1-L2:alpha"], status="PASS"),
    run_row(id="l1.gate/8/y", check="l1.gate", seams=["X:notices"], status="SKIPPED"),
    run_row(id="l4.api/t", check="l4.api", seams=["L3-L4:faked"], status="SKIPPED"),
    run_row(id="l4.api/u", check="l4.api", seams=["L3-L4:faked"], status="PASS"),
]


class Base(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="t111-matrix-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.tree = make_tree(self.tmp / "tree")
        self.map_dir = compose_map(self.tmp)
        self.map = load_map(self.map_dir)

    def results_file(self, rows: list[dict] | None = None, name: str = "results.jsonl") -> Path:
        path = self.tmp / name
        path.write_text("".join(json.dumps(r) + "\n" for r in (RESULTS if rows is None else rows)))
        return path


class Tables(Base):
    def test_sections_rows_and_strengths(self) -> None:
        text = render(self.map)
        for heading in ("## Layer 1", "## Layer 3", "## Cross-cutting", "## Planned (no code yet)", "## Totals", "## Gaps"):
            self.assertIn(heading, text)
        self.assertLess(text.index("## Layer 1"), text.index("## Layer 3"))
        self.assertLess(text.index("## Cross-cutting"), text.index("## Planned"))
        row = {ln.split("|")[1].strip(): [c.strip() for c in ln.split("|")[2:-1]] for ln in text.splitlines()
               if ln.startswith("| `")}
        self.assertEqual(row["`L1-L2:alpha`"][:3], ["1↔2", "built", "R"])
        self.assertEqual(row["`L3-L4:faked`"][:3], ["3↔4", "built", "F"])
        self.assertEqual(row["`L1-L7:stubbed`"][:3], ["1↔7", "stub", "N"])
        self.assertEqual(row["`L8-L9:later`"][:3], ["8↔9", "planned", "N"])
        self.assertEqual(row["`X:notices`"][:3], ["X", "built", "R"], "proven by a phase of l1.gate (R) and a C unit")
        self.assertIn("`l4.api` [slow, F]; `l4.ui.a` [slow, F]", row["`L3-L4:faked`"][3])
        self.assertEqual(row["`L1-L7:stubbed`"][3], "-")
        self.assertEqual(row["`L3-L4:faked`"][4], "only a fake stands in for side b")

    def test_no_last_status_column_without_results(self) -> None:
        text = render(self.map)
        self.assertNotIn("last status", text)
        self.assertNotIn(NOT_RUN, text)

    def test_totals_and_gaps(self) -> None:
        text = render(self.map)
        self.assertIn("| R running system | 2 |", text)
        self.assertIn("| F fake of the other side | 1 |", text)
        self.assertIn("| N none | 1 |", text, "the stub seam; a planned seam is not counted")
        self.assertIn("| built | 3 |", text)
        self.assertIn("| planned | 1 |", text)
        self.assertIn("| all | 5 |", text)
        self.assertIn("Built seams without running-system evidence (1 of 3): `L3-L4:faked`.", text)
        self.assertIn("Fakes checked against nothing they mirror (1 of 1): `fx/fake.txt` (L3-L4:faked).", text)

    def test_deterministic_and_markdown_tables_are_well_formed(self) -> None:
        self.assertEqual(render(self.map), render(self.map))
        for line in render(self.map).splitlines():
            if line.startswith("|") and not line.startswith("|---"):
                self.assertTrue(line.endswith("|"), line)

    def test_pipes_in_a_gap_do_not_break_a_row(self) -> None:
        self.assertEqual(matrix._cell("a | b\nc"), "a \\| b c")

    def test_best_strength_order(self) -> None:
        checks = [self.map.check(i) for i in ("l4.api", "x.fixture")]
        self.assertEqual(best_strength(c for c in checks if c), "F")
        self.assertEqual(best_strength([]), "N")
        gate = self.map.check("l1.gate")
        assert gate is not None
        self.assertEqual(best_strength([gate, *[c for c in checks if c]]), "R")


class WithResults(Base):
    def test_worst_status_per_check_and_seam(self) -> None:
        res = Results(RESULTS)
        self.assertEqual(res.status("l1.alpha"), "FAIL", "the worst of PASS, FAIL and the hygiene record")
        self.assertEqual(res.status("l4.api"), "SKIPPED")
        self.assertEqual(res.status("l4.ui.a"), NOT_RUN)
        self.assertEqual(res.status("l1.gate", "L1-L2:alpha"), "PASS", "records narrowed to the seam")
        self.assertEqual(res.status("l1.gate", "X:notices"), "SKIPPED")
        self.assertEqual(res.status("l1.gate", "L9-L9:none"), "SKIPPED", "no record of that seam: the check's worst")

    def test_render_with_results(self) -> None:
        text = render(self.map, Results(RESULTS), results_path="r.jsonl")
        self.assertIn("Last status from `r.jsonl` (run 20261111T000000Z-abc1234, head abc1234de)", text)
        self.assertIn("`l1.alpha` [fast, R] FAIL", text)
        self.assertIn("`l1.gate` [slow, R] PASS", text)
        self.assertIn("`l1.gate` [slow, R] SKIPPED; `x.fixture` [fast, C] not run", text)
        self.assertIn("`l4.api` [slow, F] SKIPPED; `l4.ui.a` [slow, F] not run", text)
        self.assertIn("checks (last status)", text)
        self.assertIn("| last status | checks |", text)
        self.assertIn("| FAIL | 1 |", text)
        self.assertIn("| not run | 3 |", text, "l4.ui.a, l5.side and x.fixture")

    def test_dirty_run_is_named(self) -> None:
        rows = [dict(RESULTS[0], dirty=True)] + RESULTS[1:]
        self.assertIn("dirty tree", render(self.map, Results(rows), results_path="r"))

    def test_read_results_round_trip(self) -> None:
        path = self.results_file()
        res = Results(read_results(path))
        self.assertEqual(len(res.records), len(RESULTS) - 1)
        self.assertEqual(res.run["head"], "abc1234def5678")

    def test_latest_results_picks_the_newest_run_dir(self) -> None:
        scratch = self.tmp / "scratch"
        for name in ("20261110T000000Z-aaa", "20261111T000000Z-bbb"):
            (scratch / name).mkdir(parents=True)
            (scratch / name / "results.jsonl").write_text("")
        (scratch / "locks").mkdir()
        self.assertEqual(matrix.latest_results(scratch), scratch / "20261111T000000Z-bbb" / "results.jsonl")
        self.assertIsNone(matrix.latest_results(self.tmp / "nope"))
        self.assertIsNone(matrix.latest_results(None))


class Cli(Base):
    def run_matrix(self, *extra: str) -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, "-I", str(RUN_PY), "--matrix", "--root", str(self.tree), "--map",
                               str(self.map_dir), *extra], cwd=self.tmp, text=True, capture_output=True, timeout=60,
                              start_new_session=True, check=False)

    def test_run_py_matrix(self) -> None:
        proc = self.run_matrix()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertTrue(proc.stdout.startswith("# Seam x evidence matrix"))
        self.assertIn("## Totals", proc.stdout)

    def test_scratch_with_a_run_brings_the_last_statuses(self) -> None:
        scratch = self.tmp / "scratch"
        (scratch / "20261111T000000Z-abc").mkdir(parents=True)
        shutil.copyfile(self.results_file(), scratch / "20261111T000000Z-abc" / "results.jsonl")
        proc = self.run_matrix("--scratch", str(scratch))
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("`l1.alpha` [fast, R] FAIL", proc.stdout)

    def test_an_unreadable_results_file_exits_2(self) -> None:
        scratch = self.tmp / "scratch"
        (scratch / "20261111T000000Z-abc").mkdir(parents=True)
        (scratch / "20261111T000000Z-abc" / "results.jsonl").write_text("{not json\n")
        proc = self.run_matrix("--scratch", str(scratch))
        self.assertEqual(proc.returncode, 2)
        self.assertIn("cannot read results", proc.stderr)

    def test_a_map_that_does_not_load_exits_2(self) -> None:
        (self.map_dir / "seams.toml").write_text("[[seam]]\nid = 'bad'\n")
        proc = self.run_matrix()
        self.assertEqual(proc.returncode, 2)
        self.assertIn("does not load", proc.stderr)


if __name__ == "__main__":
    unittest.main()
