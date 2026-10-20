#!/usr/bin/env python3
"""Tests for the native-code notices of gen-notices.py (Wave 2 review B1 m9): SQLCipher's own
LICENSE reproduced verbatim (its clauses name Zetetic LLC, so a generic BSD-3-Clause body does not
meet its binary-redistribution clause), and the OpenSSL line (copyright years from the vendored
sources, the tag-pinned LICENSE.txt, no NOTICE file to reproduce). Also checks that the committed
THIRD-PARTY-NOTICES.md carries the SQLCipher text of the libsqlite3-sys crate it was generated from.

Hermetic: synthetic crate trees in a temp dir. The committed-file check reads the crate from the
local cargo registry (scripts/check.sh runs it after cargo has fetched it) and fails when it is absent.

  python3 -I -B -m unittest discover -s 1-bootstrap/scripts/tests -p 'test_*.py'   (from os/)
"""
import contextlib
import glob
import importlib.util
import io
import os
import re
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPTS = os.path.dirname(HERE)                       # os/1-bootstrap/scripts
OS_DIR = os.path.dirname(os.path.dirname(SCRIPTS))    # os/

ZETETIC = """Copyright (c) 2008-2020 Zetetic LLC
All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions are met:
    * Neither the name of the ZETETIC LLC nor the
      names of its contributors may be used to endorse or promote products
      derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY ZETETIC LLC ''AS IS'' AND ANY
EXPRESS OR IMPLIED WARRANTIES ARE DISCLAIMED.
"""

AMALGAMATION = """/* sqlite3.c */
#define SQLITE_VERSION        "3.51.3"
#define CIPHER_VERSION_NUMBER 4.14.0
#define CIPHER_VERSION_BUILD community
"""


def load():
    spec = importlib.util.spec_from_file_location("gn_native_under_test", os.path.join(SCRIPTS, "gen-notices.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def write(path, text):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)


class SqlcipherLicense(unittest.TestCase):
    def setUp(self):
        self.gn = load()
        self.tmp = tempfile.TemporaryDirectory()
        self.crate = self.tmp.name
        write(os.path.join(self.crate, "sqlcipher", "LICENSE"), ZETETIC)
        write(os.path.join(self.crate, "sqlcipher", "sqlite3.c"), AMALGAMATION)

    def tearDown(self):
        self.tmp.cleanup()

    def refused(self, needle):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(SystemExit):
            self.gn.sqlcipher_license(self.crate)
        self.assertIn(needle, err.getvalue())

    def test_verbatim_text_copyright_and_versions(self):
        text, first, version = self.gn.sqlcipher_license(self.crate)
        self.assertEqual(text, ZETETIC.strip("\n"))
        self.assertEqual(first, "Copyright (c) 2008-2020 Zetetic LLC")
        self.assertEqual(version, "SQLCipher 4.14.0 community on SQLite 3.51.3")

    def test_missing_license_refused(self):
        os.remove(os.path.join(self.crate, "sqlcipher", "LICENSE"))
        self.refused("must be reproduced verbatim")

    def test_generic_bsd_body_refused(self):
        write(os.path.join(self.crate, "sqlcipher", "LICENSE"),
              "Copyright <copyright holders>\n\nRedistribution and use in source and binary forms\n")
        self.refused("is not the Zetetic BSD-style license")

    def test_unreadable_versions_refused(self):
        write(os.path.join(self.crate, "sqlcipher", "sqlite3.c"), "/* no defines */\n")
        self.refused("cannot read the SQLCipher / SQLite versions")


class OpensslNotice(unittest.TestCase):
    APACHE = "                                 Apache License\n                           Version 2.0, January 2004\n"

    def setUp(self):
        self.gn = load()
        self.tmp = tempfile.TemporaryDirectory()
        self.src = self.tmp.name
        root = os.path.join(self.src, "openssl")
        write(os.path.join(root, "LICENSE.txt"), self.APACHE)
        write(os.path.join(root, "crypto", "aes", "aes_core.c"),
              "/*\n * Copyright 2002-2020 The OpenSSL Project Authors. All Rights Reserved.\n */\n")
        write(os.path.join(root, "crypto", "cversion.c"),
              "/*\n * Copyright 1995-2026 The OpenSSL Project Authors. All Rights Reserved.\n */\n")
        write(os.path.join(root, "ssl", "ssl_lib.c"),
              "/*\n * Copyright 1995-2025 The OpenSSL Project Authors. All Rights Reserved.\n */\n")

    def tearDown(self):
        self.tmp.cleanup()

    def refused(self, needle):
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(SystemExit):
            self.gn.openssl_notice(self.src, "3.6.3")
        self.assertIn(needle, err.getvalue())

    def test_newest_year_and_tag_pinned_url(self):
        cp, url = self.gn.openssl_notice(self.src, "3.6.3")
        self.assertEqual(cp, "Copyright 1995-2026 The OpenSSL Project Authors. All Rights Reserved.")
        self.assertEqual(url, "https://github.com/openssl/openssl/blob/openssl-3.6.3/LICENSE.txt")

    def test_notice_file_refused(self):
        write(os.path.join(self.src, "openssl", "NOTICE"), "attribution\n")
        self.refused("ships a NOTICE file")

    def test_non_apache_license_refused(self):
        write(os.path.join(self.src, "openssl", "LICENSE.txt"), "OpenSSL License (the old dual licence)\n")
        self.refused("is not the Apache License 2.0")

    def test_no_copyright_line_refused(self):
        for f in glob.glob(os.path.join(self.src, "openssl", "*", "**", "*.c"), recursive=True):
            os.remove(f)
        self.refused("no `Copyright 1995-<year> The OpenSSL Project Authors` line")


class CommittedNotices(unittest.TestCase):
    """THIRD-PARTY-NOTICES.md reproduces the shipped crate's sqlcipher/LICENSE verbatim and names
    OpenSSL with its copyright line and tag-pinned licence URL."""

    def test_committed_file(self):
        with open(os.path.join(OS_DIR, "THIRD-PARTY-NOTICES.md"), encoding="utf-8") as f:
            notices = f.read()
        with open(os.path.join(OS_DIR, "1-bootstrap", "Cargo.lock"), encoding="utf-8") as f:
            lock = f.read()
        m = re.search(r'name = "libsqlite3-sys"\nversion = "([^"]+)"', lock)
        self.assertIsNotNone(m, "1-bootstrap/Cargo.lock has no libsqlite3-sys")
        home = os.environ.get("CARGO_HOME") or os.path.join(os.path.expanduser("~"), ".cargo")
        crates = glob.glob(os.path.join(home, "registry", "src", "*", f"libsqlite3-sys-{m.group(1)}"))
        if not crates:
            self.fail(f"libsqlite3-sys {m.group(1)} is not unpacked in {home}: run `cargo fetch` in os/1-bootstrap "
                      "(this check fails rather than skips, so it never goes silently unrun)")
        with open(os.path.join(crates[0], "sqlcipher", "LICENSE"), encoding="utf-8") as f:
            lic = f.read().strip("\n")
        self.assertIn("Copyright (c) 2008-2020 Zetetic LLC", lic)
        self.assertIn("```text\n" + lic + "\n```", notices, "SQLCipher's LICENSE is not reproduced verbatim")
        o = re.search(r'name = "openssl-src"\nversion = "[^"+]*\+([^"]+)"', lock)
        self.assertIsNotNone(o, "1-bootstrap/Cargo.lock has no openssl-src")
        ver = o.group(1)
        self.assertRegex(
            notices,
            rf"OpenSSL {re.escape(ver)} — Copyright 1995-\d{{4}} The OpenSSL Project Authors\. All Rights Reserved\. "
            rf"Apache License 2\.0 \(https://github\.com/openssl/openssl/blob/openssl-{re.escape(ver)}/LICENSE\.txt\)")


if __name__ == "__main__":
    unittest.main()
