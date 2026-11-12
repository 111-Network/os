"""``--explain <seam | check>``: the whole record of one map entry in plain text."""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from t111.explain import explain_check, explain_seam, native_command, phases_for, side_text
from t111.schema import load_map

from .helpers import RUN_PY
from .test_lint import compose_map, make_tree


class Base(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="t111-explain-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.tree = make_tree(self.tmp / "tree")
        self.map_dir = compose_map(self.tmp)
        self.map = load_map(self.map_dir)

    def seam(self, seam_id: str) -> str:
        seam = self.map.seam(seam_id)
        assert seam is not None
        return explain_seam(self.map, seam, "DIR")

    def check(self, check_id: str) -> str:
        check = self.map.check(check_id)
        assert check is not None
        return explain_check(self.map, check, "DIR")


class Seam(Base):
    def test_both_sides_with_files_and_the_read_first_marker(self) -> None:
        text = self.seam("L3-L4:faked")
        lines = text.splitlines()
        self.assertEqual(lines[0], "seam L3-L4:faked  [L3↔L4]  status: built  strength: F")
        self.assertIn("symptom: the faked seam answers 502", lines)
        self.assertIn("side a (L4):", lines)
        self.assertIn("side b (L3)  <- read first:", lines)
        self.assertIn("    fx/c.txt", lines)
        self.assertIn("    fx/d.txt", lines)
        self.assertIn("spec: fx/SPEC.md Updates & rollback; fx/SPEC.md", lines)

    def test_every_proving_check_with_tier_strength_and_both_commands(self) -> None:
        text = self.seam("L3-L4:faked")
        self.assertIn("proof (2 checks):", text)
        self.assertIn("  l4.api  unit  tier slow  strength F", text)
        self.assertIn("      run:    python3 -I verify/run.py --scratch DIR --check l4.api", text)
        self.assertIn("      native: cargo test --manifest-path 1-bootstrap/Cargo.toml -p node111 --test api_x -- "
                      "--exact '{test}'  (cost: slow)", text)
        self.assertIn("  l4.ui.a  unit  tier slow  strength F", text)
        self.assertIn("(cd 4-interface/ui && pnpm exec vitest run test/a.test.ts)", text)

    def test_phases_and_covering_gate_are_shown(self) -> None:
        text = self.seam("L1-L2:alpha")
        self.assertIn("  l1.gate  gate  tier slow  strength R", text)
        self.assertIn("      phases: 5e", text)
        self.assertIn("      native: python3 -I fx/check.py --gate  (cost: slow, ~4 min)", text)
        self.assertIn("      covered by gate l1.gate (a full run runs the gate instead)", text)
        self.assertIn("native: python3 -I fx/check.py <test>  # l1.alpha  (cost: fast, ~1 s)", text)

    def test_fakes_and_gap(self) -> None:
        text = self.seam("L3-L4:faked")
        self.assertIn("fakes (1):", text)
        self.assertIn("  fx/fake.txt  mirrors side b (L3); tests run against side a; checked against nothing it "
                      "mirrors (parity = none: a gap)", text)
        self.assertIn("gap: only a fake stands in for side b", text)
        self.assertIn("fakes: none", self.seam("L1-L2:alpha"))
        self.assertIn("gap: -", self.seam("L1-L2:alpha"))

    def test_a_seam_with_no_check_and_a_planned_seam(self) -> None:
        stub = self.seam("L1-L7:stubbed")
        self.assertIn("proof: none", stub)
        self.assertIn("(no files yet)", stub)
        planned = self.seam("L8-L9:later")
        self.assertIn("no code yet: a planned seam", planned)
        self.assertIn("status: planned  strength: N", planned)

    def test_cross_cutting_layers(self) -> None:
        self.assertEqual(self.seam("X:notices").splitlines()[0], "seam X:notices  [X]  status: built  strength: R")
        self.assertEqual(side_text(self.map.seam("X:notices").a), "X")  # type: ignore[union-attr]

    def test_parity_check_wording(self) -> None:
        from dataclasses import replace
        fake = replace(self.map.fakes[0], parity="l1.alpha")
        smap = replace(self.map, fakes=(fake,))
        seam = smap.seam("L3-L4:faked")
        assert seam is not None
        self.assertIn("parity check l1.alpha", explain_seam(smap, seam, "DIR"))


class CheckRecord(Base):
    def test_unit_record(self) -> None:
        text = self.check("l4.api")
        lines = text.splitlines()
        self.assertEqual(lines[0], "check l4.api  unit  tier slow  strength F  [L3↔L4]")
        self.assertTrue(lines[1].startswith("cmd: cargo test --manifest-path"), lines[1])
        self.assertEqual(lines[2], "run: python3 -I verify/run.py --scratch DIR --check l4.api")
        self.assertIn("adapter exit  timeout 600 s  locks cargo-slot  needs -  requires -", text)
        self.assertIn("proves 1 seam(s):", text)
        self.assertIn("  L3-L4:faked  [L3↔L4]  built", text)
        self.assertIn("known: l4.api/* euid0 fails (fx/SPEC.md:1): fails as root", text)

    def test_gate_and_its_units(self) -> None:
        gate = self.check("l1.gate")
        self.assertIn("covers 1 unit(s): l1.alpha", gate)
        self.assertIn("proves 2 seam(s):", gate)
        self.assertIn("  L1-L2:alpha  [L1↔L2]  built  (phases 5e)", gate)
        self.assertIn("  X:notices  [X]  built  (phases 8)", gate)
        unit = self.check("l1.alpha")
        self.assertIn("covered by gate l1.gate", unit)

    def test_needs_cwd_and_env(self) -> None:
        text = self.check("l4.ui.a")
        self.assertIn("needs pnpm", text)
        self.assertIn("cmd: (cd 4-interface/ui && pnpm exec vitest run test/a.test.ts)", text)

    def test_parity_check_lists_the_fake(self) -> None:
        from dataclasses import replace
        fake = replace(self.map.fakes[0], parity="l1.alpha")
        smap = replace(self.map, fakes=(fake,))
        check = smap.check("l1.alpha")
        assert check is not None
        self.assertIn("parity check of: fx/fake.txt (L3-L4:faked)", explain_check(smap, check, "DIR"))

    def test_a_check_that_proves_nothing(self) -> None:
        self.assertIn("proves no seam", self.check("l5.side"))

    def test_native_command_forms(self) -> None:
        self.assertEqual(native_command(self.map.check("l1.alpha")),  # type: ignore[arg-type]
                         "python3 -I fx/check.py <test>  # l1.alpha")
        self.assertEqual(phases_for(self.map.check("l1.gate"), "X:notices"), ["8"])  # type: ignore[arg-type]


class Cli(Base):
    def explain(self, ident: str, *extra: str) -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, "-I", str(RUN_PY), "--root", str(self.tree), "--map", str(self.map_dir),
                               "--explain", ident, *extra], cwd=self.tmp, text=True, capture_output=True, timeout=60,
                              start_new_session=True, check=False)

    def test_run_py_explain_a_seam_and_a_check(self) -> None:
        seam = self.explain("L3-L4:faked", "--scratch", "/tmp/elsewhere")
        self.assertEqual(seam.returncode, 0, seam.stderr)
        self.assertIn("run:    python3 -I verify/run.py --scratch /tmp/elsewhere --check l4.api", seam.stdout)
        check = self.explain("l1.gate")
        self.assertEqual(check.returncode, 0, check.stderr)
        self.assertIn("covers 1 unit(s): l1.alpha", check.stdout)

    def test_an_unknown_id_exits_2_with_the_closest(self) -> None:
        proc = self.explain("L3-L4:fake")
        self.assertEqual(proc.returncode, 2)
        self.assertIn("no seam or check with id 'L3-L4:fake'", proc.stderr)
        self.assertIn("closest: L3-L4:faked", proc.stderr)
        self.assertEqual(proc.stdout, "")
        none = self.explain("zzzz")
        self.assertEqual(none.returncode, 2)
        self.assertNotIn("closest", none.stderr)

    def test_a_map_that_does_not_load_exits_2(self) -> None:
        (self.map_dir / "checks.toml").write_text("[[check]]\n")
        proc = self.explain("l1.gate")
        self.assertEqual(proc.returncode, 2)
        self.assertIn("does not load", proc.stderr)

    def test_one_command_at_a_time(self) -> None:
        proc = subprocess.run([sys.executable, "-I", str(RUN_PY), "--root", str(self.tree), "--map",
                               str(self.map_dir), "--explain", "l1.gate", "--lint"], cwd=self.tmp, text=True,
                              capture_output=True, timeout=60, start_new_session=True, check=False)
        self.assertEqual(proc.returncode, 2)

    def test_output_is_short_enough_to_read_in_ten_seconds(self) -> None:
        for ident in ("L3-L4:faked", "L1-L2:alpha", "l1.gate"):
            self.assertLess(len(self.explain(ident).stdout.splitlines()), 30, ident)


if __name__ == "__main__":
    unittest.main()
