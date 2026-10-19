"""T0 scaffold check: the sidecar layout the spec names (§6) is present. C3 adds the real tests."""
import os
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)


class Layout(unittest.TestCase):
    def test_layout(self):
        for name in ("README.md", "requirements.in", "run-tests.sh"):
            self.assertTrue(os.path.isfile(os.path.join(ROOT, name)), name)

    def test_requirements_pin_rns(self):
        with open(os.path.join(ROOT, "requirements.in"), encoding="utf-8") as f:
            pins = [l.strip() for l in f if l.strip() and not l.startswith("#")]
        self.assertTrue(any(p.startswith("rns==") for p in pins), pins)
        self.assertTrue(any(p.startswith("lxmf==") for p in pins), pins)


if __name__ == "__main__":
    unittest.main()
