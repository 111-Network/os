# SPDX-License-Identifier: GPL-3.0-or-later
"""The hash-locked venv file and the panel declaration (spec sections 13 and 14).

No network. The lock is checked structurally (what ``pip install --require-hashes`` demands);
``python3 gen_lock.py --verify`` is the same check from the command line.
"""
import importlib.util
import os
import re
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
OS_DIR = os.path.abspath(os.path.join(ROOT, "..", "..", ".."))
sys.path.insert(0, ROOT)

LOCK = os.path.join(ROOT, "meshtastic-presence-lock.txt")
REQ_IN = os.path.join(ROOT, "requirements.in")
PANEL = os.path.join(OS_DIR, "1-bootstrap", "crates", "node111", "assets", "panels", "meshtastic.toml")
SPEC = os.path.join(OS_DIR, "5-messaging", "README.md")

_spec = importlib.util.spec_from_file_location("gen_lock", os.path.join(ROOT, "gen_lock.py"))
gen_lock = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gen_lock)


class LockTests(unittest.TestCase):
    def test_lock_is_complete_pinned_and_hashed(self):
        lock = gen_lock.verify(LOCK, REQ_IN)
        self.assertEqual(lock["meshtastic"]["version"], "2.7.11")
        for name, entry in lock.items():
            self.assertGreaterEqual(len(entry["hashes"]), 1, name)

    def test_lock_has_no_urls_vcs_or_options(self):
        with open(LOCK, encoding="utf-8") as f:
            for line in f:
                body = line.strip()
                if not body or body.startswith("#") or body.startswith("--hash="):
                    continue
                self.assertRegex(body, r"^[a-z0-9][a-z0-9._-]*==\S+( ; .+?)?( \\)?$", line)
                self.assertNotRegex(body, r"(git\+|https?://|file:|-e |--index|--find-links|--extra-index)")

    def test_the_closure_contains_what_meshtastic_needs_and_is_small(self):
        lock = gen_lock.verify(LOCK, REQ_IN)
        for needed in ("meshtastic", "protobuf", "pypubsub", "pyserial", "pyyaml", "requests", "tabulate",
                       "packaging"):
            self.assertIn(needed, lock)
        self.assertLessEqual(len(lock), 30)
        # no Reticulum anywhere (spec section 1: Reticulum is a connected service, never a dependency)
        self.assertFalse([n for n in lock if n in ("rns", "lxmf", "nomadnet")])

    def test_requirements_in_pins_exactly_meshtastic(self):
        self.assertEqual(gen_lock.read_requirements_in(REQ_IN), ["meshtastic==2.7.11"])

    def test_parse_lock_refuses_what_pip_would_refuse(self):
        h = "a" * 64
        ok = "pkg==1.0 \\\n    --hash=sha256:%s\n" % h
        self.assertEqual(gen_lock.parse_lock(ok)["pkg"]["hashes"], [h])
        for bad in ("pkg>=1.0 \\\n    --hash=sha256:%s\n" % h,          # not an exact pin
                    "pkg==1.0\n",                                           # no hash
                    "pkg==1.0 \\\n    --hash=sha256:short\n",               # malformed hash
                    "    --hash=sha256:%s\n" % h,                           # hash without requirement
                    ok + ok,                                                # duplicate
                    "pkg==1.0 \\\n    --hash=sha256:%s \\\n    --hash=sha256:%s\n" % (h, h),  # repeated hash
                    "",
                    "git+https://example.invalid/x.git#egg=x\n"):
            with self.assertRaises(ValueError, msg=bad):
                gen_lock.parse_lock(bad)

    def test_marker_logic_never_narrows(self):
        self.assertIsNone(gen_lock.marker_for([("a", None), ("b", 'python_version < "3.11"')]))
        self.assertEqual(gen_lock.marker_for([("a", 'python_version < "3.11"')]), 'python_version < "3.11"')
        self.assertEqual(gen_lock.marker_for([("a", "m1"), ("b", "m2")]), "(m1) or (m2)")
        self.assertIsNone(gen_lock.marker_for([]))

    def test_edges_ignore_extras_and_outside_packages(self):
        installs = [
            {"metadata": {"name": "Root", "version": "1", "requires_dist": [
                "dep>=1", "cond>=1; python_version < '3.11'", "extra-only>=1; extra == 'cli'", "outside>=1"]}},
            {"metadata": {"name": "dep", "version": "1", "requires_dist": ["cond; python_version < '3.11'"]}},
            {"metadata": {"name": "cond", "version": "1"}},
            {"metadata": {"name": "extra_only", "version": "1"}},
        ]
        e = gen_lock.edges(installs)
        self.assertEqual(e["dep"], [("root", None)])
        self.assertEqual(sorted(e["cond"]), [("dep", "python_version < '3.11'"), ("root", "python_version < '3.11'")])
        self.assertEqual(e["extra-only"], [])


def parse_toml_subset(text):
    """Line-grammar parser for the subset the panel declaration uses (py < 3.11 has no tomllib).

    Tables, ``key = value`` with basic strings (no escapes), integers, booleans and one-line
    arrays of basic strings, and ``#`` comments outside strings. Raises ValueError otherwise.
    """
    root, cur = {}, None
    cur = root
    seen_tables = set()

    def strip_comment(line):
        in_str = False
        for i, ch in enumerate(line):
            if ch == '"':
                in_str = not in_str
            elif ch == "#" and not in_str:
                return line[:i]
        if in_str:
            raise ValueError("unterminated string")
        return line

    def value(tok):
        tok = tok.strip()
        if re.fullmatch(r'"[^"\\]*"', tok):
            return tok[1:-1]
        if tok in ("true", "false"):
            return tok == "true"
        if re.fullmatch(r"[+-]?[0-9]+", tok):
            return int(tok)
        if tok.startswith("[") and tok.endswith("]"):
            inner = tok[1:-1].strip()
            if not inner:
                return []
            if not re.fullmatch(r'("[^"\\]*"\s*,\s*)*"[^"\\]*"\s*,?', inner):
                raise ValueError("bad array %r" % tok)
            return re.findall(r'"([^"\\]*)"', inner)
        raise ValueError("bad value %r" % tok)

    for no, raw in enumerate(text.splitlines(), 1):
        line = strip_comment(raw).strip()
        if not line:
            continue
        m = re.fullmatch(r"\[([A-Za-z0-9_-]+)\]", line)
        if m:
            if m.group(1) in seen_tables:
                raise ValueError("line %d: duplicate table" % no)
            seen_tables.add(m.group(1))
            cur = root.setdefault(m.group(1), {})
            continue
        m = re.fullmatch(r"([A-Za-z0-9_-]+)\s*=\s*(.+)", line)
        if not m:
            raise ValueError("line %d: not key = value: %r" % (no, raw))
        if m.group(1) in cur:
            raise ValueError("line %d: duplicate key" % no)
        cur[m.group(1)] = value(m.group(2))
    return root


EXPECTED = {
    "id": "meshtastic", "title": "Meshtastic", "kb_entry": "projects/meshtastic-web",
    "source": {"kind": "static", "bundle": "panels/meshtastic-web", "shim": "meshtastic-http"},
    "entry": {"path": "/"},
    "health": {"listen": 11230},
    "frame": {"allow": ["serial", "bluetooth"], "needs_secure_context": False, "sandbox": "default",
              "secure_note": "USB or Bluetooth from this browser works only on the node itself and only for a radio "
                             "this node has not bound; the bound board is reached through the panel's HTTP connection."},
    "proxy": {"websocket": False, "sse": False, "max_body": "8MiB"},
    "offline": {"optional_hosts": ["tiles.openfreemap.org", "api.meshtastic.org"]},
    "auth": {"mode": "111-session"},
    "license": {"spdx": "GPL-3.0-only", "served": "unmodified", "source_offer": "THIRD-PARTY-NOTICES.md"},
}


class PanelManifestTests(unittest.TestCase):
    def read(self):
        with open(PANEL, encoding="utf-8") as f:
            return f.read()

    def test_line_grammar_parse_equals_the_expected_declaration(self):
        self.assertEqual(parse_toml_subset(self.read()), EXPECTED)

    def test_real_toml_parser_agrees_when_one_exists(self):
        try:
            import tomllib as toml  # Python 3.11+
        except ImportError:
            try:
                import tomli as toml  # type: ignore[no-redef]
            except ImportError:
                self.skipTest("no TOML parser in this interpreter; the line-grammar parser above ran")
        self.assertEqual(toml.loads(self.read()), EXPECTED)

    def test_the_file_is_the_spec_section_13_block_verbatim(self):
        with open(SPEC, encoding="utf-8") as f:
            spec = f.read()
        blocks = re.findall(r"```toml\n(.*?)```", spec, re.S)
        self.assertIn(self.read(), blocks)

    def test_v1_invariants(self):
        d = parse_toml_subset(self.read())
        self.assertEqual(d["source"]["kind"], "static")           # v1 implements static only
        self.assertFalse(d["proxy"]["websocket"] or d["proxy"]["sse"])  # true fails adoption in v1
        self.assertEqual(d["health"]["listen"], 11230)            # meshmud TCP downlink
        self.assertEqual(d["license"]["spdx"], "GPL-3.0-only")
        self.assertEqual(d["license"]["served"], "unmodified")
        self.assertIn(d["auth"]["mode"], ("111-session", "loopback-only"))
        self.assertTrue(11190 > d["health"]["listen"] or d["health"]["listen"] > 11229)  # not a panel slot


class SubsetParserTests(unittest.TestCase):
    def test_rejects_what_toml_rejects(self):
        for bad in ("a = ", "a = 'single'", 'a = "unterminated', "[t]\n[t]\n", "a = 1\na = 2\n", "a b = 1",
                    'a = ["x", 5]', "a = nope"):
            with self.assertRaises(ValueError, msg=bad):
                parse_toml_subset(bad)

    def test_accepts_comments_and_arrays(self):
        self.assertEqual(parse_toml_subset('x = ["a#b", "c"]  # trailing\n[t]\ny = true\n'),
                         {"x": ["a#b", "c"], "t": {"y": True}})


if __name__ == "__main__":
    unittest.main()
