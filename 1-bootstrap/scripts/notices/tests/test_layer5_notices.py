#!/usr/bin/env python3
"""Tests for the Layer 5 paths of gen-notices.py (Go closure, Python venv lock, pins).

Hermetic: every test works on a COPY of the real inputs in a temp tree laid out like os/, so
nothing here touches the real notices. No network, no cargo, no pnpm, no go.

  python3 -I -m unittest discover -s 1-bootstrap/scripts/notices/tests      (from os/)
"""
import contextlib
import importlib.util
import io
import os
import re
import shutil
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.dirname(os.path.dirname(HERE))          # os/1-bootstrap/scripts
OS_DIR = os.path.dirname(os.path.dirname(SCRIPTS))        # os/
PRESENCE = os.path.join("5-messaging", "sidecars", "meshtastic-presence")


def copy_tree(dst):
    """A minimal os/ tree holding only the Layer 5 inputs."""
    for rel in ("1-bootstrap/scripts/gen-notices.py", "1-bootstrap/scripts/meshmud-build.sh",
                "1-bootstrap/scripts/meshtastic-web-build.sh",
                PRESENCE + "/meshtastic-presence-lock.txt", PRESENCE + "/LICENSE"):
        os.makedirs(os.path.dirname(os.path.join(dst, rel)), exist_ok=True)
        shutil.copy(os.path.join(OS_DIR, rel), os.path.join(dst, rel))
    shutil.copytree(os.path.join(SCRIPTS, "notices"), os.path.join(dst, "1-bootstrap/scripts/notices"),
                    ignore=shutil.ignore_patterns("tests", "__pycache__"))


def load(dst):
    spec = importlib.util.spec_from_file_location("gn_under_test", os.path.join(dst, "1-bootstrap/scripts/gen-notices.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = self.tmp.name
        copy_tree(self.root)
        self.gn = load(self.root)

    def tearDown(self):
        self.tmp.cleanup()

    def path(self, rel):
        return os.path.join(self.root, rel)

    def edit(self, rel, fn):
        p = self.path(rel)
        with open(p, encoding="utf-8") as f:
            text = f.read()
        new = fn(text)
        self.assertNotEqual(new, text, "the edit changed nothing")
        with open(p, "w", encoding="utf-8") as f:
            f.write(new)

    def dies(self, fn, *needles):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(SystemExit) as cm:
            fn()
        self.assertEqual(cm.exception.code, 1)
        for n in needles:
            self.assertIn(n, err.getvalue())
        return err.getvalue()


class GoClosureTests(Base):
    LIC = "1-bootstrap/scripts/notices/go-licenses.txt"
    MOD = "1-bootstrap/scripts/notices/meshmud-v1.0.0.go.mod"

    def test_the_real_inputs_are_consistent(self):
        go = self.gn.collect_go()
        self.assertEqual(len(go["closure"]), 14)
        self.assertEqual(go["main"][0], "github.com/Rikonardo/meshmud")
        self.assertEqual(go["std"][0], "std")
        self.assertEqual(go["pins"]["MESHMUD_TAG"], "v1.0.0")

    def test_a_module_without_a_license_row_is_refused(self):
        self.edit(self.LIC, lambda t: re.sub(r"(?m)^github\.com/samber/lo \|.*\n", "", t))
        self.dies(self.gn.collect_go, "github.com/samber/lo", "no row")

    def test_a_version_bump_in_go_mod_without_the_license_row_is_refused(self):
        self.edit(self.MOD, lambda t: t.replace("github.com/samber/lo v1.53.0", "github.com/samber/lo v1.54.0"))
        self.dies(self.gn.collect_go, "github.com/samber/lo v1.54.0")

    def test_a_stale_license_row_is_refused(self):
        self.edit(self.LIC, lambda t: t + "github.com/example/gone | v0.1.0 | MIT | Copyright (c) X | https://example.invalid\n")
        self.dies(self.gn.collect_go, "github.com/example/gone")

    def test_missing_go_sum_hash_is_refused(self):
        self.edit("1-bootstrap/scripts/notices/meshmud-v1.0.0.go.sum",
                  lambda t: "".join(l for l in t.splitlines(True) if not l.startswith("github.com/samber/lo v1.53.0 h1:")))
        self.dies(self.gn.collect_go, "go.sum", "github.com/samber/lo")

    def test_malformed_row_is_refused(self):
        self.edit(self.LIC, lambda t: t + "github.com/example/short | v1.0.0 | MIT\n")
        self.dies(self.gn.collect_go, "expected 5 non-empty columns")

    def test_go_mod_parsing(self):
        parse = self.gn.parse_go_mod
        self.assertEqual(parse("module m\n\ngo 1.24\n\nrequire a/b v1.0.0\nrequire (\n\tc/d v2.0.0 // indirect\n\te/f v3.0.0\n)\n"),
                         [("a/b", "v1.0.0", False), ("c/d", "v2.0.0", True), ("e/f", "v3.0.0", False)])
        self.dies(lambda: parse("module m\nreplace a => b v1\n"), "replace")
        self.dies(lambda: parse("module m\nrequire (\n a v1\n"), "unterminated")
        self.dies(lambda: parse("module m\nrequire (\n a\n)\n"), "cannot read")

    def test_unknown_license_id_needs_a_text(self):
        self.dies(lambda: self.gn.check_license_ids([("x", "1", "WTFPL", "c", "r")], "list", {"MIT"}), "WTFPL")
        self.gn.check_license_ids([("x", "1", "MIT OR Apache-2.0", "c", "r")], "list", {"MIT", "Apache-2.0"})


class PythonLockTests(Base):
    LIC = "1-bootstrap/scripts/notices/python-licenses.txt"
    LOCK = PRESENCE + "/meshtastic-presence-lock.txt"

    def test_the_real_inputs_are_consistent(self):
        py = self.gn.collect_python()
        self.assertEqual(len(py["rows"]), 16)
        self.assertEqual(py["lock"]["meshtastic"][0], "2.7.11")
        self.assertIn("python_full_version", py["lock"]["async-timeout"][1])

    def test_a_locked_package_without_a_row_is_refused(self):
        self.edit(self.LIC, lambda t: re.sub(r"(?m)^tabulate \|.*\n", "", t))
        self.dies(self.gn.collect_python, "tabulate", "no row")

    def test_a_version_bump_in_the_lock_is_refused(self):
        self.edit(self.LOCK, lambda t: t.replace("tabulate==0.9.0", "tabulate==0.9.1"))
        self.dies(self.gn.collect_python, "tabulate==0.9.1", "says 0.9.0")

    def test_a_stale_row_is_refused(self):
        self.edit(self.LIC, lambda t: t + "leftpad | 1.0 | MIT | Copyright (c) X | https://example.invalid\n")
        self.dies(self.gn.collect_python, "leftpad", "not in the lock")

    def test_a_new_locked_package_is_refused_until_it_has_a_row(self):
        self.edit(self.LOCK, lambda t: t + "\nnewpkg==1.0 \\\n    --hash=sha256:" + "a" * 64 + "\n")
        self.dies(self.gn.collect_python, "newpkg==1.0")

    def test_name_normalisation(self):
        self.edit(self.LIC, lambda t: t.replace("typing-extensions |", "Typing_Extensions |"))
        self.gn.collect_python()

    def test_lock_parser(self):
        lock = self.gn.parse_lock("# c\nfoo-bar==1.0 ; python_version < '3.11' \\\n    --hash=sha256:" + "b" * 64 + "\nBaz==2 \\\n    --hash=sha256:" + "c" * 64 + "\n")
        self.assertEqual(lock, {"foo-bar": ("1.0", "python_version < '3.11'"), "baz": ("2", None)})


class PinTests(Base):
    def test_pins_are_read_from_the_scripts(self):
        mp = self.gn.meshmud_pins()
        self.assertRegex(mp["MESHMUD_COMMIT"], r"^[0-9a-f]{40}$")
        self.assertRegex(mp["MESHMUD_TARBALL_SHA256"], r"^[0-9a-f]{64}$")
        wp = self.gn.web_pins()
        self.assertRegex(wp["WEB_COMMIT"], r"^[0-9a-f]{40}$")
        self.assertRegex(wp["WEB_ASSET_SHA256"], r"^[0-9a-f]{64}$")
        self.assertRegex(wp["WEB_TREE_SHA256"], r"^[0-9a-f]{64}$")

    def test_a_missing_pin_is_refused(self):
        self.edit("1-bootstrap/scripts/meshmud-build.sh", lambda t: re.sub(r'(?m)^MESHMUD_COMMIT=.*\n', "", t))
        self.dies(self.gn.meshmud_pins, "MESHMUD_COMMIT")

    def test_changing_a_pin_changes_its_digest_and_other_script_edits_do_not(self):
        before = self.gn.pins_digest(self.gn.meshmud_pins())
        self.edit("1-bootstrap/scripts/meshmud-build.sh", lambda t: t.replace("# CI ONLY.", "# CI ONLY (edited comment)."))
        self.assertEqual(self.gn.pins_digest(self.gn.meshmud_pins()), before)
        self.edit("1-bootstrap/scripts/meshmud-build.sh", lambda t: t.replace('MESHMUD_TAG="v1.0.0"', 'MESHMUD_TAG="v1.0.1"'))
        self.assertNotEqual(self.gn.pins_digest(self.gn.meshmud_pins()), before)

    def test_input_records_cover_every_layer5_input(self):
        labels = [rel for _h, rel in self.gn.input_records()]
        for needed in ("1-bootstrap/scripts/meshmud-build.sh#pins", "1-bootstrap/scripts/meshtastic-web-build.sh#pins",
                       "1-bootstrap/scripts/notices/meshmud-v1.0.0.go.mod", "1-bootstrap/scripts/notices/meshmud-v1.0.0.go.sum",
                       "1-bootstrap/scripts/notices/go-licenses.txt", "1-bootstrap/scripts/notices/python-licenses.txt",
                       PRESENCE + "/meshtastic-presence-lock.txt", PRESENCE + "/LICENSE"):
            self.assertIn(needed, labels)


if __name__ == "__main__":
    unittest.main()
