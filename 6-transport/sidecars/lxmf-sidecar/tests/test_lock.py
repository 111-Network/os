"""The hash-locked venv file (spec section 6): structure, pins, the embedded component copy.

No network. ``python3 gen_lock.py --verify`` is the same check from the command line.
"""
import importlib.util
import os
import re
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
OS_DIR = os.path.abspath(os.path.join(ROOT, "..", "..", ".."))

LOCK = os.path.join(ROOT, "lxmf-sidecar-lock.txt")
REQ_IN = os.path.join(ROOT, "requirements.in")
COMPONENT = os.path.join(OS_DIR, "10-bootstrap", "crates", "node111", "assets", "components", "lxmf.lock.txt")
MANIFEST = os.path.join(OS_DIR, "10-bootstrap", "crates", "node111", "assets", "components", "lxmf.toml")

_spec = importlib.util.spec_from_file_location("gen_lock", os.path.join(ROOT, "gen_lock.py"))
gen_lock = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen_lock)


class LockTests(unittest.TestCase):
    def test_lock_is_complete_pinned_and_hashed(self):
        lock = gen_lock.verify(LOCK, REQ_IN, COMPONENT)
        for name, entry in lock.items():
            self.assertGreaterEqual(len(entry["hashes"]), 1, name)

    def test_rns_and_lxmf_are_pinned_to_the_requirements(self):
        pins = dict(p.split("==") for p in gen_lock.read_requirements_in(REQ_IN))
        self.assertEqual(set(pins), {"rns", "lxmf"})
        lock = gen_lock.verify(LOCK, REQ_IN)
        self.assertEqual(lock["rns"]["version"], pins["rns"])
        self.assertEqual(lock["lxmf"]["version"], pins["lxmf"])

    def test_closure_is_the_expected_small_set(self):
        lock = gen_lock.verify(LOCK, REQ_IN)
        self.assertEqual(set(lock), {"rns", "lxmf", "cryptography", "cffi", "pycparser", "pyserial"})

    def test_lock_has_no_urls_vcs_or_options(self):
        with open(LOCK, encoding="utf-8") as f:
            for line in f:
                body = line.strip()
                if not body or body.startswith("#") or body.startswith("--hash="):
                    continue
                self.assertRegex(body, r"^[a-z0-9][a-z0-9._-]*==\S+( ; .+?)?( \\)?$", line)
                self.assertNotRegex(body, r"(git\+|https?://|file:|-e |--index|--find-links|--extra-index)")

    @unittest.skipUnless(os.path.exists(COMPONENT), "the assets copy lands with C3rs (the Rust adapter)")
    def test_component_copy_is_byte_identical_and_names_match_the_manifest(self):
        with open(LOCK, "rb") as a, open(COMPONENT, "rb") as b:
            self.assertEqual(a.read(), b.read())
        with open(MANIFEST, encoding="utf-8") as f:
            self.assertRegex(f.read(), r'(?m)^lock\s*=\s*"lxmf\.lock\.txt"')

    @unittest.skipUnless(os.path.exists(COMPONENT), "the assets copy lands with C3rs (the Rust adapter)")
    def test_component_lock_hashes_are_the_manifest_parsers_view(self):
        # the Rust side counts every `--hash=sha256:<64 hex>` token; the lock must yield some
        with open(COMPONENT, encoding="utf-8") as f:
            tokens = re.findall(r"--hash=sha256:([0-9a-f]{64})", f.read())
        self.assertGreater(len(tokens), 6)
        self.assertEqual(len(tokens), len(set(tokens)))

    def test_requirements_in_pins_exactly(self):
        for req in gen_lock.read_requirements_in(REQ_IN):
            self.assertRegex(req, r"^[A-Za-z0-9._-]+==[0-9][^\s;]*$")

    def test_parse_lock_refuses_what_pip_would_refuse(self):
        h = "a" * 64
        ok = "pkg==1.0 \\\n    --hash=sha256:%s\n" % h
        self.assertEqual(gen_lock.parse_lock(ok)["pkg"]["hashes"], [h])
        for bad in ("pkg>=1.0 \\\n    --hash=sha256:%s\n" % h,
                    "pkg==1.0\n",
                    "    --hash=sha256:%s\n" % h,
                    "pkg==1.0 \\\n    --hash=sha256:%s \\\n    --hash=sha256:%s\n" % (h, h),
                    "pkg==1.0 \\\n    --hash=sha256:%s\npkg==1.1 \\\n    --hash=sha256:%s\n" % (h, "b" * 64),
                    "",
                    "pkg==1.0 \\\n    --hash=sha256:nothex\n"):
            with self.assertRaises(ValueError, msg=bad):
                gen_lock.parse_lock(bad)

    def test_verify_detects_drift_in_the_component_copy(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            drift = os.path.join(d, "lxmf.lock.txt")
            with open(LOCK, encoding="utf-8") as f:
                text = f.read()
            with open(drift, "w", encoding="utf-8") as f:
                f.write(text + "\n# edited\n")
            with self.assertRaises(ValueError):
                gen_lock.verify(LOCK, REQ_IN, drift)

    def test_verify_rejects_a_lock_that_does_not_pin_requirements(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            req = os.path.join(d, "requirements.in")
            with open(req, "w", encoding="utf-8") as f:
                f.write("rns==9.9.9\nlxmf==1.2.1\n")
            with self.assertRaises(ValueError):
                gen_lock.verify(LOCK, req)

    def test_render_from_a_canned_pip_report(self):
        installs = [
            {"requested": True, "metadata": {"name": "lxmf", "version": "1.0", "requires_dist": ["rns>=1"]}},
            {"requested": True, "metadata": {"name": "rns", "version": "2.0", "requires_dist": ["cffi; platform_system == 'Linux'"]}},
            {"requested": False, "metadata": {"name": "cffi", "version": "3.0", "requires_dist": []}},
        ]
        orig = gen_lock.pypi_hashes
        gen_lock.pypi_hashes = lambda name, version: ["c" * 64, "d" * 64]
        try:
            text = gen_lock.render(installs, "x, CPython 3.11")
        finally:
            gen_lock.pypi_hashes = orig
        lock = gen_lock.parse_lock(text)
        self.assertEqual(set(lock), {"lxmf", "rns", "cffi"})
        self.assertEqual(lock["cffi"]["marker"], "platform_system == 'Linux'")  # only conditional edges: keeps it
        self.assertIsNone(lock["rns"]["marker"])


if __name__ == "__main__":
    unittest.main()
