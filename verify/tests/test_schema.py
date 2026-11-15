"""The map's data model and its loader: the fixture loads; each schema error names file, table, index, field."""

from __future__ import annotations

import tempfile
import tomllib
import unittest
from pathlib import Path

from t111 import schema
from t111.schema import SchemaError, load_map, parse_map

from .helpers import FIXTURE_MAP

GOOD_CHECK = {"id": "a.b", "kind": "unit", "cmd": ["true"], "tier": "fast", "layers": [1], "strength": "I",
              "timeout_s": 5}
GOOD_SEAM = {"id": "L1-L2:x", "status": "built", "a": {"layer": 1, "files": []}, "b": {"layer": 2, "files": []},
             "read_first": "a"}


def errors_of(seams: dict, checks: dict) -> list[str]:
    try:
        parse_map(seams, checks)
    except SchemaError as exc:
        return [str(e) for e in exc.errors]
    return []


class Load(unittest.TestCase):
    def test_fixture_loads(self) -> None:
        m = load_map(FIXTURE_MAP)
        self.assertEqual(len(m.seams), 3)
        self.assertIsNotNone(m.check("fx.pass"))
        fx_fail = m.check("fx.fail")
        assert fx_fail is not None
        self.assertEqual(fx_fail.adapter, "exit")
        self.assertEqual(fx_fail.cwd, ".")
        self.assertEqual(m.check("fx.covered").covered_by, "fx.gate")  # type: ignore[union-attr]
        self.assertEqual(m.check("fx.verify1").locks(), ("verify",))  # type: ignore[union-attr]
        self.assertEqual(m.check("fx.ns").locks(), ("verify",))  # type: ignore[union-attr]
        self.assertEqual(m.known[0].words(), "fails as root in the fixture")
        self.assertEqual(m.unmapped[0].path, "fx/unrelated.sh")
        seam = m.seam("L3-L4:fx-faked")
        assert seam is not None
        self.assertEqual(seam.layers(), (3, 4))
        self.assertEqual(m.fakes_on(seam.id)[0].mirrors, "b")

    def test_side_layer_forms(self) -> None:
        seam = dict(GOOD_SEAM, id="L0-L5/6/7:entries", a={"layer": 0, "files": ["x"]}, b={"layer": [5, 6, 7]})
        x = dict(GOOD_SEAM, id="X:notices", a={"layer": "X"}, b={"layer": "X"})
        m = parse_map({"seam": [seam, x]}, {})
        self.assertEqual(m.seams[0].b.layer, (5, 6, 7))
        self.assertEqual(m.seams[0].b.to_json(), {"layer": [5, 6, 7], "files": []})
        self.assertEqual(m.seams[1].layers(), ("X",))

    def test_missing_file_and_bad_toml(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            d = Path(tmp)
            (d / "seams.toml").write_text("[[seam]\n")
            with self.assertRaises(SchemaError) as ctx:
                load_map(d, display_root=d)
            text = str(ctx.exception)
            self.assertIn("seams.toml: not valid TOML", text)
            self.assertIn("checks.toml: file not found", text)


class Errors(unittest.TestCase):
    def test_check_field_errors(self) -> None:
        bad = dict(GOOD_CHECK, tier="medium", strength="Z", timeout_s=0, cargo="yes", colour="red", cmd=[])
        errs = errors_of({}, {"check": [GOOD_CHECK, bad]})
        want = [
            "verify/checks.toml: check[1] (a.b).colour: unknown field",
            "verify/checks.toml: check[1] (a.b).tier: 'medium' is not one of fast|slow|ns",
            "verify/checks.toml: check[1] (a.b).strength: 'Z' is not one of R|I|F|C|N",
            "verify/checks.toml: check[1] (a.b).timeout_s: must be >= 1, got 0",
            "verify/checks.toml: check[1] (a.b).cargo: must be true or false, got str",
            "verify/checks.toml: check[1] (a.b).cmd: must not be empty",
        ]
        for w in want:
            self.assertTrue(any(e.startswith(w) for e in errs), f"{w!r} not in {errs}")

    def test_missing_required(self) -> None:
        errs = errors_of({"seam": [{"id": "L1-L2:y"}]}, {"check": [{"id": "c"}]})
        for fld in ("status", "a", "b", "read_first"):
            self.assertIn(f"verify/seams.toml: seam[0] (L1-L2:y).{fld}: missing (required)", errs)
        for fld in ("kind", "cmd", "tier", "layers", "strength", "timeout_s"):
            self.assertIn(f"verify/checks.toml: check[0] (c).{fld}: missing (required)", errs)

    def test_ids_and_references(self) -> None:
        gate = dict(GOOD_CHECK, id="g", kind="gate")
        unit = dict(GOOD_CHECK, id="u", covered_by="u2", requires=["nope"])
        unit2 = dict(GOOD_CHECK, id="u2", covered_by="g")
        dup = dict(GOOD_CHECK, id="g", kind="gate")
        slash = dict(GOOD_CHECK, id="x/y")
        errs = errors_of({"seam": [GOOD_SEAM, GOOD_SEAM, dict(GOOD_SEAM, id="bad id")]},
                         {"check": [gate, unit, unit2, dup, slash]})
        self.assertIn("verify/seams.toml: seam[1] (L1-L2:x).id: duplicate id (first at seam[0])", errs)
        self.assertTrue(any("seam[2] (bad id).id: 'bad id' is not a seam id" in e for e in errs), errs)
        self.assertIn("verify/checks.toml: check[3] (g).id: duplicate id (first at check[0])", errs)
        self.assertIn("verify/checks.toml: check[1] (u).covered_by: 'u2' is a unit, not a gate", errs)
        self.assertIn("verify/checks.toml: check[1] (u).requires: names no check: 'nope'", errs)
        self.assertTrue(any("check[4] (x/y).id" in e for e in errs), errs)

    def test_gate_has_no_cover_and_cycles(self) -> None:
        a = dict(GOOD_CHECK, id="a", requires=["b"])
        b = dict(GOOD_CHECK, id="b", requires=["a"])
        g = dict(GOOD_CHECK, id="g", kind="gate", covered_by="a")
        errs = errors_of({}, {"check": [a, b, g]})
        self.assertIn("verify/checks.toml: check: requires: cycle: a -> b -> a".replace("check: requires",
                                                                                         "check.requires"), errs)
        self.assertTrue(any("check[2] (g).covered_by: a gate is covered by nothing" in e for e in errs), errs)

    def test_other_tables(self) -> None:
        errs = errors_of({"seams": []}, {
            "fake": [{"path": "p", "mirrors": "c", "seam": "s"}],
            "known": [{"test": "t", "condition": "sometimes", "source": "x", "kind": "odd"}],
            "unmapped": {"a/b.sh": ""},
            "extra": 1,
        })
        self.assertIn("verify/seams.toml: seams: unknown top-level table (allowed: seam)", errs)
        self.assertIn("verify/checks.toml: extra: unknown top-level table (allowed: check, fake, known, unmapped)",
                      errs)
        self.assertIn("verify/checks.toml: fake[0].mirrors: 'c' is not one of a|b", errs)
        self.assertIn("verify/checks.toml: known[0].condition: 'sometimes' is not one of euid0|nonroot", errs)
        self.assertIn("verify/checks.toml: known[0].kind: 'odd' is not one of fails|vacuous", errs)
        self.assertIn("verify/checks.toml: unmapped.a/b.sh: needs a reason (a non-empty string)", errs)

    def test_every_field_is_documented(self) -> None:
        doc = schema.Check.__doc__ or ""
        for name in schema.Check.__dataclass_fields__:
            self.assertIn(f"``{name}``", doc, name)
        for cls in (schema.Seam, schema.Side, schema.Fake, schema.Known, schema.Unmapped):
            for name in cls.__dataclass_fields__:
                self.assertIn(f"``{name}``", cls.__doc__ or "", f"{cls.__name__}.{name}")

    def test_fixture_is_valid_toml(self) -> None:
        with (FIXTURE_MAP / "checks.toml").open("rb") as fh:
            self.assertIn("check", tomllib.load(fh))


if __name__ == "__main__":
    unittest.main()
