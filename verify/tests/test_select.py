"""Selection: full runs, --gate, --layer, --seam, --check, tiers, covered units, requires ordering."""

from __future__ import annotations

import unittest

from t111.schema import load_map
from t111.select import Selection, SelectionError, select

from .helpers import FIXTURE_MAP

MAP = load_map(FIXTURE_MAP)


def ids(sel: Selection) -> list[str]:
    return [c.id for c in select(MAP, sel).checks]


class Select(unittest.TestCase):
    def test_full_run_defaults(self) -> None:
        got = ids(Selection())
        self.assertNotIn("fx.ns", got, "ns is opt-in")
        self.assertIn("fx.gate", got)
        self.assertNotIn("fx.covered", got, "a unit whose gate runs does not run twice")
        self.assertNotIn("fx.slowcovered", got)
        self.assertIn("fx.slowgate", got)
        tiers = [MAP.check(i).tier for i in got]  # type: ignore[union-attr]
        self.assertEqual(tiers, sorted(tiers, key=["fast", "slow", "ns"].index), "fast, then slow")
        self.assertLess(got.index("fx.fail"), got.index("fx.requires"))

    def test_unit_runs_alone_when_its_gate_is_outside_the_tiers(self) -> None:
        got = ids(Selection(tiers=("fast",)))
        self.assertIn("fx.slowcovered", got)
        self.assertNotIn("fx.slowgate", got)

    def test_unit_replaced_by_its_gate(self) -> None:
        plan = select(MAP, Selection(layers=("0",)))
        self.assertEqual([c.id for c in plan.checks], ["fx.gate"])
        self.assertIn("fx.covered: replaced by its covering gate fx.gate", plan.notes)

    def test_gate_flag(self) -> None:
        got = ids(Selection(gate=True))
        self.assertTrue(all(MAP.check(i).kind == "gate" for i in got), got)  # type: ignore[union-attr]
        self.assertEqual(ids(Selection(checks=("fx.covered",), gate=True)), ["fx.gate"])

    def test_check_any_tier_unless_tier_given(self) -> None:
        self.assertEqual(ids(Selection(checks=("fx.ns",))), ["fx.ns"])
        with self.assertRaises(SelectionError):
            select(MAP, Selection(checks=("fx.ns",), tiers=("fast",)))
        self.assertEqual(ids(Selection(checks=("fx.covered",))), ["fx.covered"], "--check runs the unit itself")

    def test_seam(self) -> None:
        plan = select(MAP, Selection(seams=("L1-L2:fx-alpha",), tiers=("fast",)))
        got = [c.id for c in plan.checks]
        self.assertIn("fx.pass", got)
        self.assertNotIn("fx.gate", got, "units first; a gate only where no unit covers the seam")
        self.assertNotIn("fx.ns", got)
        self.assertIn("fx.ns", plan.omitted)
        faked = ids(Selection(seams=("L3-L4:fx-faked",)))
        self.assertEqual(faked[:2], ["fx.fail", "fx.requires"])
        self.assertIn("fx.covered", faked)
        self.assertIn("fx.gate", ids(Selection(seams=("L3-L4:fx-faked",), gate=True)))

    def test_requires_pulled_and_ordered(self) -> None:
        plan = select(MAP, Selection(checks=("fx.requires",)))
        self.assertEqual([c.id for c in plan.checks], ["fx.fail", "fx.requires"])
        self.assertEqual(plan.pulled, {"fx.fail"})

    def test_unknowns(self) -> None:
        for sel in (Selection(checks=("nope",)), Selection(seams=("L1-L2:nope",)), Selection(tiers=("medium",)),
                    Selection(layers=("12",)), Selection(seams=("L8-L9:fx-planned",))):
            with self.assertRaises(SelectionError, msg=str(sel)):
                select(MAP, sel)


if __name__ == "__main__":
    unittest.main()
