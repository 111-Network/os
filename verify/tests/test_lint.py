"""The map lint: a clean map passes, one mutated map per rule fails exactly that rule, the warnings warn.

The maps live in ``fixtures/lint/<name>/``: ``clean`` holds both files, every other directory holds the ONE file it
mutates and borrows the other from ``clean``. The tree they describe is built by ``make_tree`` (also used by the
tests of ``--matrix``, ``--changed`` and ``--explain``).
"""

from __future__ import annotations

import argparse
import io
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from dataclasses import replace
from pathlib import Path

from t111 import lint
from t111.lint import (RULES, SCHEMA_RULE, check_files, claimed_files, glob_match, glob_paths, lint_dir, matches_entry,
                       records, render, section_matches, split_spec, verify_hits)
from t111.schema import load_map

from .helpers import RUN_PY, VERIFY

LINT_FIXTURES = VERIFY / "tests" / "fixtures" / "lint"

SPEC_MD = """# Spec

## 1. Intro

text

### 2.1 Part

## Updates & rollback

```
# 9.9 a comment inside a code fence, not a heading
```
"""

TREE: dict[str, str] = {
    "fx/SPEC.md": SPEC_MD,
    "fx/a.txt": "a\n",
    "fx/b.txt": "b\n",
    "fx/c.txt": "c\n",
    "fx/d.txt": "d\n",
    "fx/fake.txt": "a fake\n",
    "fx/stub.rs": "//! Layer 7 (§1): T0 stub; filled by a later row.\n",
    "fx/check.py": "print('ok')\n",
    "fx/gen/x.txt": "generated\n",
    "1-bootstrap/Cargo.toml": "[workspace]\n",
    "1-bootstrap/crates/node111/tests/api_x.rs": "// a test file\n",
    "1-bootstrap/crates/node111/tests/skip_one.rs": "// unmapped by glob\n",
    "1-bootstrap/manifest/gen.py": "print('manifest')\n",
    "1-bootstrap/scripts/cross-build.sh": "#!/bin/sh\nexit 0\n",
    "4-interface/ui/test/a.test.ts": "// vitest file\n",
    "5-messaging/sidecars/side/run-tests.sh": "#!/bin/sh\nexit 0\n",
    "3-intelligence/verify/helper.sh": "#!/bin/sh\nexit 0\n",
}
"""The files of the stand-in tree the fixture maps describe (path -> text)."""


def make_tree(root: Path, extra: dict[str, str] | None = None) -> Path:
    """Write ``TREE`` (and ``extra``, which wins) under ``root``; returns ``root``."""
    files = dict(TREE)
    files.update(extra or {})
    for rel, text in files.items():
        path = root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")
    return root


def compose_map(dest: Path, name: str | None = None) -> Path:
    """``dest/map``: the clean map with the files of fixture ``name`` laid over it."""
    out = dest / "map"
    out.mkdir(parents=True, exist_ok=True)
    for fname in ("seams.toml", "checks.toml"):
        src = LINT_FIXTURES / "clean" / fname
        if name and (LINT_FIXTURES / name / fname).is_file():
            src = LINT_FIXTURES / name / fname
        shutil.copyfile(src, out / fname)
    return out


class Base(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="t111-lint-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.tree = make_tree(self.tmp / "tree")

    def lint(self, name: str | None = None, **kw) -> lint.LintResult:
        return lint_dir(compose_map(self.tmp, name), self.tree, **kw)


class Clean(Base):
    def test_clean_map_passes(self) -> None:
        result = self.lint()
        self.assertEqual(result.fails(), [])
        self.assertTrue(result.loaded)
        self.assertEqual(result.failing_rules(), [])
        self.assertEqual((result.n_seams, result.n_checks, result.n_fakes), (5, 6, 1))
        self.assertEqual(result.n_candidates, 5)

    def test_clean_map_has_exactly_the_fake_parity_warning(self) -> None:
        result = self.lint()
        self.assertEqual([w.rule for w in result.warns()], ["fake-parity"])
        self.assertIn("1 of 1 fake(s)", result.warns()[0].message)

    def test_rule_ids_are_stable(self) -> None:
        self.assertEqual([r for r, _ in RULES],
                         ["ids", "files", "unknown-seam", "unclaimed", "built-gap", "fake-mirror", "pi-floor", "known"])
        self.assertEqual(sorted(lint.RULE_FUNCS), sorted(r for r, _ in RULES))
        self.assertEqual(lint.WARNINGS, ("stub-marker", "fake-parity"))


class Mutations(Base):
    def assertFailsOnly(self, name: str, rule: str, *words: str) -> lint.LintResult:
        result = self.lint(name)
        self.assertEqual(result.failing_rules(), [rule], [f"{f.rule}: {f.subject}: {f.message}" for f in result.fails()])
        text = " | ".join(f"{f.subject}: {f.message}" for f in result.fails(rule))
        for word in words:
            self.assertIn(word, text)
        return result

    def test_ids_duplicate(self) -> None:
        result = self.assertFailsOnly("ids-duplicate", "ids", "duplicate seam id", "seam[5] repeats seam[0]")
        self.assertTrue(result.loaded, "the later duplicate is dropped so every other rule still runs")
        self.assertEqual([f.subject for f in result.fails()], ["L1-L2:alpha"])

    def test_ids_do_not_match_their_layers(self) -> None:
        self.assertFailsOnly("ids-layers", "ids", "side b 5", "names layers 1 and 2")

    def test_check_id_does_not_match_its_layers(self) -> None:
        self.assertFailsOnly("ids-check-layers", "ids", "layer 4", "[6]")

    def test_missing_side_file(self) -> None:
        self.assertFailsOnly("files-side", "files", "fx/no-such-file.txt")

    def test_spec_section_without_a_heading(self) -> None:
        self.assertFailsOnly("files-spec-section", "files", "no heading for §9.9")

    def test_spec_heading_text_without_a_heading(self) -> None:
        self.assertFailsOnly("files-spec-heading", "files", "Nothing of the kind")

    def test_missing_check_script(self) -> None:
        self.assertFailsOnly("files-script", "files", "fx/gone.py")

    def test_unknown_seam(self) -> None:
        self.assertFailsOnly("unknown-seam", "unknown-seam", "L9-L9:ghost")

    def test_unknown_seam_in_phases(self) -> None:
        text = (LINT_FIXTURES / "clean" / "checks.toml").read_text().replace('"8" = ["X:notices"]', '"8" = ["X:notices", "X:nope"]')
        m = compose_map(self.tmp)
        (m / "checks.toml").write_text(text)
        result = lint_dir(m, self.tree)
        self.assertEqual(result.failing_rules(), ["unknown-seam"])
        self.assertIn("phases['8']", result.fails()[0].message)

    def test_verify_script_claimed_by_nothing_and_not_unmapped(self) -> None:
        self.assertFailsOnly("unclaimed", "unclaimed", "no check claims this file")
        self.assertEqual([f.subject for f in self.lint("unclaimed").fails()], ["3-intelligence/verify/helper.sh"])

    def test_every_rule_has_a_mutation_that_trips_it_and_the_clean_map_that_does_not(self) -> None:
        tripped = {"ids": "ids-layers", "files": "files-side", "unknown-seam": "unknown-seam",
                   "unclaimed": "unclaimed", "built-gap": "built-gap", "fake-mirror": "fake-mirror",
                   "known": "known-check"}
        for rule, name in tripped.items():
            self.assertEqual(self.lint(name).failing_rules(), [rule], name)
        make_tree(self.tree, {"1-bootstrap/manifest/gen.py": "run('verify/run.py')\n"})
        self.assertEqual(self.lint().failing_rules(), ["pi-floor"])
        make_tree(self.tree)
        self.assertEqual(self.lint().failing_rules(), [])
        self.assertEqual(sorted([*tripped, "pi-floor"]), sorted(r for r, _ in RULES))

    def test_built_seam_with_no_check_and_no_gap(self) -> None:
        self.assertFailsOnly("built-gap", "built-gap", "no check proves it")

    def test_built_seam_without_r_needs_a_gap(self) -> None:
        self.assertFailsOnly("built-gap-weak", "built-gap", "no check of strength R (best: F)")

    def test_literal_reading_of_rule_5_accepts_a_seam_with_only_weak_checks(self) -> None:
        self.assertEqual(self.lint("built-gap-weak", gap_needs_r=False).fails(), [])
        self.assertEqual(self.lint("built-gap", gap_needs_r=False).failing_rules(), ["built-gap"])

    def test_fake_names_a_missing_mirror(self) -> None:
        result = self.assertFailsOnly("fake-mirror", "fake-mirror", "fx/no-fake.txt", "l9.no-such-check")
        self.assertEqual(len(result.fails()), 2)

    def test_fake_mirrors_a_side_with_no_files(self) -> None:
        text = (LINT_FIXTURES / "clean" / "checks.toml").read_text().replace('seam = "L3-L4:faked"', 'seam = "L1-L7:stubbed"')
        m = compose_map(self.tmp)
        (m / "checks.toml").write_text(text)
        result = lint_dir(m, self.tree)
        self.assertEqual(result.failing_rules(), ["fake-mirror"])
        self.assertIn("lists no files", result.fails()[0].message)

    def test_schema_error_is_reported_not_raised(self) -> None:
        m = compose_map(self.tmp)
        (m / "seams.toml").write_text('[[seam]]\nid = "L1-L2:x"\nstatus = "fine"\n')
        result = lint_dir(m, self.tree)
        self.assertFalse(result.loaded)
        self.assertEqual(result.failing_rules(), [SCHEMA_RULE])
        self.assertTrue(any("status" in f.message for f in result.fails()))

    def test_missing_and_broken_map_files(self) -> None:
        m = compose_map(self.tmp)
        (m / "seams.toml").unlink()
        (m / "checks.toml").write_text("[[check\n")
        result = lint_dir(m, self.tree)
        self.assertEqual(result.failing_rules(), [SCHEMA_RULE])
        self.assertEqual(len(result.fails()), 2)


class Ids(Base):
    def test_a_check_id_with_no_layer_prefix_fails(self) -> None:
        for bad in ("smoke5", "n1.side5", "side5"):
            text = (LINT_FIXTURES / "clean" / "checks.toml").read_text().replace('id = "l5.side"', f'id = "{bad}"')
            m = compose_map(self.tmp)
            (m / "checks.toml").write_text(text)
            result = lint_dir(m, self.tree)
            self.assertEqual(result.failing_rules(), ["ids"], bad)
            self.assertEqual([f.subject for f in result.fails()], [bad])
            self.assertIn("no layer prefix", result.fails()[0].message)

    def test_x_and_l_prefixes_still_pass(self) -> None:
        self.assertEqual(self.lint().fails("ids"), [], "l1.gate ... x.fixture are the clean map's ids")


class Known(Base):
    def lint_known(self, **fields: str) -> lint.LintResult:
        text = (LINT_FIXTURES / "clean" / "checks.toml").read_text()
        block = "".join(f'{k} = "{v}"\n' for k, v in fields.items())
        m = compose_map(self.tmp)
        (m / "checks.toml").write_text(text.replace('[[known]]\ntest = "l4.api/*"\ncondition = "euid0"\n'
                                                    'source = "fx/SPEC.md:1"\ncheck = "l4.api"\n',
                                                    "[[known]]\ncondition = \"euid0\"\n" + block))
        return lint_dir(m, self.tree)

    def test_the_clean_known_passes(self) -> None:
        self.assertEqual(self.lint_known(test="l4.api/*", source="fx/SPEC.md:1", check="l4.api").fails(), [])

    def test_a_check_that_names_no_check_fails(self) -> None:
        result = self.lint("known-check")
        self.assertEqual(result.failing_rules(), ["known"])
        self.assertIn("'l4.apx'", result.fails()[0].message)

    def test_a_vacuous_entry_with_a_typo_in_its_check_prefix_fails(self) -> None:
        result = self.lint_known(test="l1.gte/x/*", source="fx/SPEC.md:1", kind="vacuous")
        self.assertEqual(result.failing_rules(), ["known"])
        self.assertIn("check prefix 'l1.gte'", result.fails()[0].message)

    def test_a_wildcard_prefix_needs_one_match(self) -> None:
        self.assertEqual(self.lint_known(test="*/preflight_happy", source="fx/SPEC.md:1").fails(), [])
        self.assertEqual(self.lint_known(test="l1.*/x", source="fx/SPEC.md:1").fails(), [])
        self.assertEqual(self.lint_known(test="zz.*/x", source="fx/SPEC.md:1").failing_rules(), ["known"])

    def test_a_source_that_is_not_a_file_fails(self) -> None:
        for source in ("fx/NO-SUCH.md:80", "fx/NO-SUCH.md", "fx", "owner said so"):
            result = self.lint_known(test="l4.api/*", source=source)
            self.assertEqual(result.failing_rules(), ["known"], source)
            self.assertIn("is not a file of the tree", result.fails()[0].message)

    def test_a_source_with_a_line_range_and_words_passes(self) -> None:
        self.assertEqual(self.lint_known(test="l4.api/*", source="fx/SPEC.md:1-9 (the spec)").fails(), [])


class Unclaimed(Base):
    def test_a_new_verify_script_nobody_claims_fails(self) -> None:
        make_tree(self.tree, {"4-interface/verify/new.sh": "#!/bin/sh\nexit 0\n"})
        result = self.lint()
        self.assertEqual(result.failing_rules(), ["unclaimed"])
        self.assertEqual([f.subject for f in result.fails()], ["4-interface/verify/new.sh"])

    def test_every_candidate_glob_is_scanned(self) -> None:
        extra = {"2-identity/crates/id111/tests/t.rs": "", "5-messaging/crates/mesh111/tests/t.rs": "",
                 "3-intelligence/crates/agent111/tests/t.rs": "", "6-transport/sidecars/s/run-tests.sh": "",
                 "4-interface/ui/test/b.test.ts": "", "4-interface/ui/test/helpers/x.test.ts": "",
                 "7-relay/verify/g.sh": ""}
        make_tree(self.tree, extra)
        got = {f.subject for f in self.lint().fails("unclaimed")}
        self.assertEqual(got, {p for p in extra if "helpers" not in p})

    def test_the_verify_glob_sees_one_level_deeper(self) -> None:
        make_tree(self.tree, {"6-transport/verify/spine/node.sh": "", "6-transport/verify/devices/fake.sh": "",
                              "6-transport/verify/spine/deep/too.sh": ""})
        got = {f.subject for f in self.lint().fails("unclaimed")}
        self.assertEqual(got, {"6-transport/verify/spine/node.sh", "6-transport/verify/devices/fake.sh"},
                         "two levels down is where the glob stops")

    def test_unmapped_entry_by_path_and_by_glob(self) -> None:
        make_tree(self.tree, {"3-intelligence/verify/h2.sh": "", "1-bootstrap/crates/node111/tests/skip_two.rs": ""})
        got = {f.subject for f in self.lint().fails("unclaimed")}
        self.assertEqual(got, {"3-intelligence/verify/h2.sh"}, "skip_two.rs is covered by the skip_*.rs glob")

    def test_an_unmapped_path_that_matches_nothing_fails_files(self) -> None:
        (self.tree / "3-intelligence/verify/helper.sh").unlink()
        result = self.lint()
        self.assertEqual(result.failing_rules(), ["files"])
        self.assertIn("matches nothing", result.fails()[0].message)

    def test_cargo_check_claims_by_test_name_package_and_scope(self) -> None:
        m = load_map(compose_map(self.tmp))
        cands = ["1-bootstrap/crates/node111/tests/api_x.rs", "1-bootstrap/crates/node111/tests/other.rs",
                 "1-bootstrap/crates/mesh111/tests/api_x.rs"]
        api = m.check("l4.api")
        assert api is not None
        self.assertEqual(claimed_files(api, self.tree, cands) & set(cands), {cands[0]},
                         "--test api_x of package node111 only")
        from dataclasses import replace
        whole = replace(api, cmd=("cargo", "test", "-p", "node111"))
        self.assertEqual(claimed_files(whole, self.tree, cands) & set(cands), set(cands[:2]),
                         "no --test: the package's tests")
        scoped = replace(api, cmd=("cargo", "test", "--manifest-path", "1-bootstrap/Cargo.toml"))
        self.assertEqual(claimed_files(scoped, self.tree, cands) & set(cands), set(cands), "no -p: the manifest's tree")
        bare = replace(api, cmd=("cargo", "test"))
        self.assertEqual(claimed_files(bare, self.tree, cands), set(), "cwd . and no selector: nothing")

    def test_cargo_lib_doc_bin_and_example_runs_claim_no_integration_test(self) -> None:
        m = load_map(compose_map(self.tmp))
        api = m.check("l4.api")
        assert api is not None
        cands = ["1-bootstrap/crates/node111/tests/api_x.rs", "1-bootstrap/crates/node111/tests/other.rs"]
        base = ("cargo", "test", "--manifest-path", "1-bootstrap/Cargo.toml", "-p", "node111")
        for extra in (("--lib",), ("--doc",), ("--bin", "node111"), ("--bins",), ("--bin=node111",), ("--example", "e"),
                      ("--examples",), ("--example=e",), ("--lib", "--", "--exact", "x")):
            self.assertEqual(claimed_files(replace(api, cmd=base + extra), self.tree, cands) & set(cands), set(),
                             " ".join(extra))
        for extra in (("--lib", "--test", "api_x"), ("--lib", "--tests"), ("--test", "api_x")):
            got = claimed_files(replace(api, cmd=base + extra), self.tree, cands) & set(cands)
            self.assertEqual(got, {cands[0]} if "--tests" not in extra else set(cands), " ".join(extra))
        self.assertEqual(claimed_files(replace(api, cmd=("cargo", "test", "--lib"), cwd="1-bootstrap"), self.tree,
                                       cands), set(), "a cwd scope with --lib claims nothing either")

    def test_a_lib_unit_leaves_the_integration_file_unclaimed(self) -> None:
        text = (LINT_FIXTURES / "clean" / "checks.toml").read_text().replace(
            '"--test", "api_x", "--", "--exact", "{test}"', '"--lib"')
        m = compose_map(self.tmp)
        (m / "checks.toml").write_text(text)
        result = lint_dir(m, self.tree)
        self.assertEqual(result.failing_rules(), ["unclaimed"])
        self.assertEqual([f.subject for f in result.fails()], ["1-bootstrap/crates/node111/tests/api_x.rs"])

    def test_a_directory_token_claims_the_candidates_below_it(self) -> None:
        from dataclasses import replace
        m = load_map(compose_map(self.tmp))
        ui = m.check("l4.ui.a")
        assert ui is not None
        whole = replace(ui, cmd=("pnpm", "exec", "vitest", "run", "test"))
        self.assertEqual(claimed_files(whole, self.tree, ["4-interface/ui/test/a.test.ts", "5-x/y.sh"]),
                         {"4-interface/ui/test/a.test.ts"})
        dot = replace(ui, cmd=("pnpm", "exec", "vitest", "run", "."))
        self.assertEqual(claimed_files(dot, self.tree, ["4-interface/ui/test/a.test.ts"]), set(), "'.' claims nothing")

    def test_check_files_skip_options_placeholders_and_args_after_dashdash(self) -> None:
        m = load_map(compose_map(self.tmp))
        api = m.check("l4.api")
        fixture = m.check("x.fixture")
        assert api is not None and fixture is not None
        self.assertEqual([t for t, _ in check_files(api, self.tree)], ["1-bootstrap/Cargo.toml"])
        self.assertEqual([p for _, p in check_files(fixture, self.tree)], [self.tree / "fx" / "check.py"],
                         "{root} expands, {scratch} paths are skipped")


class CmdTokens(Base):
    def one_check(self, cmd: tuple[str, ...]):
        m = load_map(compose_map(self.tmp))
        fixture = m.check("x.fixture")
        assert fixture is not None
        return m, replace(fixture, cmd=cmd)

    def files_failures(self, cmd: tuple[str, ...]) -> list[str]:
        m, check = self.one_check(cmd)
        smap = replace(m, checks=tuple(check if c.id == check.id else c for c in m.checks))
        return [f.message for f in lint.rule_files(smap, self.tree) if f.subject == check.id]

    def test_a_glob_token_is_a_pattern_not_a_missing_file(self) -> None:
        for token in ("fx/test_*.py", "fx/test_?.py", "fx/test_[ab].py"):
            self.assertEqual(self.files_failures(("python3", "-I", "-m", "unittest", token)), [], token)
            m, check = self.one_check(("python3", token))
            self.assertEqual(check_files(check, self.tree), [], token)
        self.assertEqual(self.files_failures(("python3", "--glob=fx/*.py")), [])
        self.assertEqual(len(self.files_failures(("python3", "fx/gone.py"))), 1, "a plain missing file still fails")

    def test_a_glob_token_claims_nothing(self) -> None:
        m, check = self.one_check(("python3", "fx/*.py"))
        self.assertEqual(claimed_files(check, self.tree, ["fx/check.py"]), set())

    def test_the_script_of_a_sh_c_token_is_checked(self) -> None:
        self.assertEqual(len(self.files_failures(("sh", "-c", "fx/gone.sh --smoke"))), 1)
        self.assertIn("fx/gone.sh", self.files_failures(("sh", "-c", "fx/gone.sh --smoke"))[0])
        self.assertEqual(len(self.files_failures(("bash", "-lc", "fx/gone.py && echo done"))), 1)
        self.assertEqual(self.files_failures(("sh", "-c", "python3 -I fx/check.py --gate")), [],
                         "first word is a command, not a path")
        self.assertEqual(self.files_failures(("sh", "-c", "fx/check.py --gate")), [], "it exists")
        self.assertEqual(self.files_failures(("python3", "-c", "import os; print(os.getcwd())")), [])
        self.assertEqual(self.files_failures(("sh", "-c", "$ROOT/gone.sh --x")), [], "a variable is not resolved")
        self.assertEqual(self.files_failures(("sh", "-c", "{root}/fx/check.py --x")), [], "{root} expands")
        self.assertEqual(len(self.files_failures(("sh", "-c", "{root}/fx/gone.sh --x"))), 1)

    def test_a_sh_c_token_with_no_space_is_the_plain_token_rule(self) -> None:
        self.assertEqual(len(self.files_failures(("sh", "-c", "fx/gone.sh"))), 1, "no duplicate report")


class PiFloor(Base):
    def test_manifest_naming_a_verify_path_fails(self) -> None:
        make_tree(self.tree, {"1-bootstrap/manifest/gen.py": "import os\nrun('verify/run.py')\n"})
        result = self.lint()
        self.assertEqual(result.failing_rules(), ["pi-floor"])
        self.assertEqual(result.fails()[0].subject, "1-bootstrap/manifest/gen.py:2")

    def test_cross_build_naming_a_verify_path_fails(self) -> None:
        make_tree(self.tree, {"1-bootstrap/scripts/cross-build.sh": "#!/bin/sh\nsh \"$ROOT/verify/x.sh\"\n"})
        result = self.lint()
        self.assertEqual([f.subject for f in result.fails("pi-floor")], ["1-bootstrap/scripts/cross-build.sh:2"])

    def test_verify_hits(self) -> None:
        hit = ["python3 verify/run.py", "x = './verify/t111/a.py'", "cp ../verify/x .", "\"${REPO}/verify/y\"",
               "$ROOT/verify/z", "a=verify/q"]
        miss = ["4-interface/verify/smoke.sh", "my-verify/x", "6-transport/verify/lib.sh", "the verify folder",
                "verify-all/x", "verify/"]
        for line in hit:
            self.assertEqual(len(verify_hits(line)), 1, line)
        for line in miss:
            self.assertEqual(verify_hits(line), [], line)

    def test_the_spellings_the_review_found(self) -> None:
        hit = ['cp -r "$ROOT"/verify/run.py x', 'cp -r "$ROOT/verify" "$OUT/"', "cp -r ../verify dist/",
               'os.path.join(root, "verify", "run.py")', 'cp "$(git rev-parse --show-toplevel)/verify/run.py" x',
               "cp -r `git rev-parse --show-toplevel`/verify x", "cp -r verify dist", "tar czf x.tgz verify/",
               "rsync -a verify/ out/", "p = Path(root) / 'verify'", 'cd "$ROOT"/verify', "ls ${ROOT}/verify",
               "x = f'{root}/verify/run.py'", 'python3 "${HERE}/../verify/run.py"']
        miss = ['cp -r "$ROOT"/6-transport/verify/lib.sh x', 'os.path.join(root, "6-transport", "verify")',
                "the verify directory is not shipped", "verify.sh", "echo verify", 'x = "verify"', "cp -r everyverify x",
                "cp -r 4-interface/verify x", 'cp "$ROOT/4-interface/verify/smoke.sh" x']
        for line in hit:
            self.assertEqual(len(verify_hits(line)), 1, line)
        for line in miss:
            self.assertEqual(verify_hits(line), [], line)

    def test_each_spelling_fails_the_rule_in_a_script(self) -> None:
        for line in ('cp -r "$ROOT/verify" "$OUT/"', 'cp "$(git rev-parse --show-toplevel)/verify/run.py" x',
                     "cp -r ../verify dist/"):
            make_tree(self.tree, {"1-bootstrap/scripts/cross-build.sh": f"#!/bin/sh\n{line}\n"})
            self.assertEqual([f.subject for f in self.lint().fails("pi-floor")],
                             ["1-bootstrap/scripts/cross-build.sh:2"], line)
        make_tree(self.tree, {"1-bootstrap/manifest/gen.py": 'p = os.path.join(root, "verify", "run.py")\n'})
        self.assertEqual([f.subject for f in self.lint().fails("pi-floor")], ["1-bootstrap/manifest/gen.py:1"])

    def test_a_pi_floor_path_missing_from_the_tree_fails(self) -> None:
        (self.tree / "1-bootstrap/scripts/cross-build.sh").unlink()
        result = self.lint()
        self.assertEqual(result.failing_rules(), ["pi-floor"])
        self.assertEqual([f.subject for f in result.fails()], ["1-bootstrap/scripts/cross-build.sh"])
        self.assertIn("missing from the tree", result.fails()[0].message)
        shutil.rmtree(self.tree / "1-bootstrap/manifest")
        self.assertEqual([f.subject for f in self.lint().fails("pi-floor")],
                         ["1-bootstrap/manifest", "1-bootstrap/scripts/cross-build.sh"])

    def test_a_layer_verify_path_is_fine(self) -> None:
        make_tree(self.tree, {"1-bootstrap/manifest/gen.py": "see 4-interface/verify/smoke.sh\n"})
        self.assertEqual(self.lint().fails(), [])

    def test_the_whole_manifest_directory_is_searched(self) -> None:
        make_tree(self.tree, {"1-bootstrap/manifest/deep/dir/list.txt": "verify/seams.toml\n"})
        self.assertEqual([f.subject for f in self.lint().fails()], ["1-bootstrap/manifest/deep/dir/list.txt:1"])


class Globs(Base):
    CASES = [
        ("fx/*.txt", "fx/a.txt", True), ("fx/*.txt", "fx/gen/x.txt", False), ("fx/*", "fx/gen/x.txt", False),
        ("1-bootstrap/*/api_x.rs", "1-bootstrap/crates/node111/tests/api_x.rs", False),
        ("1-bootstrap/**/api_x.rs", "1-bootstrap/crates/node111/tests/api_x.rs", True),
        ("1-bootstrap/**/api_x.rs", "1-bootstrap/api_x.rs", True), ("**/api_x.rs", "api_x.rs", True),
        ("fx/**", "fx/gen/x.txt", True), ("fx/**", "fx", True), ("fx/?.txt", "fx/a.txt", True),
        ("fx/?.txt", "fx/ab.txt", False), ("fx/[ab].txt", "fx/b.txt", True), ("fx/[ab].txt", "fx/c.txt", False),
        ("fx/a*", "fx/a.txt", True), ("a*b", "a/b", False), ("fx/*.txt", "fx", False),
    ]

    def test_glob_match_table(self) -> None:
        for pattern, path, want in self.CASES:
            self.assertEqual(glob_match(pattern, path), want, f"{pattern} vs {path}")

    def test_glob_paths_and_glob_match_agree_on_the_tree(self) -> None:
        every = sorted({p.relative_to(self.tree).as_posix() for p in self.tree.rglob("*")})
        for pattern in ["fx/*.txt", "fx/*", "fx/**", "**/*.rs", "1-bootstrap/**/api_x.rs", "1-bootstrap/*/api_x.rs",
                        "[0-9]-*/verify/*.sh", "fx/?.txt", "fx/[ab].txt", "*/*/*/*/api_x.rs", "no/such/*.txt",
                        "fx/**/x.txt", "**"]:
            walked = sorted({p.relative_to(self.tree).as_posix() for p in glob_paths(self.tree, pattern)} - {"."})
            matched = sorted(p for p in every if glob_match(pattern, p))
            self.assertEqual(walked, matched, pattern)

    def test_a_glob_entry_also_matches_below_a_directory_it_matches(self) -> None:
        self.assertTrue(matches_entry("fx/g*", "fx/gen/x.txt"))
        self.assertTrue(matches_entry("fx/gen", "fx/gen/x.txt"))
        self.assertFalse(matches_entry("fx/*.txt", "fx/gen/x.txt"))
        self.assertTrue(matches_entry("fx/**/x.txt", "fx/gen/x.txt"))

    def test_files_and_unclaimed_read_one_entry_the_same_way(self) -> None:
        make_tree(self.tree, {"1-bootstrap/crates/node111/tests/skip_two.rs": ""})
        for entry, files_fail, unclaimed in [("1-bootstrap/*/skip_*.rs", True, {"skip_one.rs", "skip_two.rs"}),
                                             ("1-bootstrap/crates/node111/tests/skip_*.rs", False, set()),
                                             ("1-bootstrap/**/skip_*.rs", False, set())]:
            text = (LINT_FIXTURES / "clean" / "checks.toml").read_text().replace(
                '"1-bootstrap/crates/node111/tests/skip_*.rs"', f'"{entry}"')
            m = compose_map(self.tmp)
            (m / "checks.toml").write_text(text)
            result = lint_dir(m, self.tree)
            self.assertEqual(bool(result.fails("files")), files_fail, entry)
            self.assertEqual({f.subject.rsplit("/", 1)[-1] for f in result.fails("unclaimed")}, unclaimed, entry)


class Warnings(Base):
    def test_stub_without_its_marker_warns(self) -> None:
        make_tree(self.tree, {"fx/stub.rs": "//! Layer 7: the real thing now.\n"})
        result = self.lint()
        self.assertEqual(result.fails(), [])
        warns = result.warns("stub-marker")
        self.assertEqual(len(warns), 1)
        self.assertIn("promote the row", warns[0].message)
        self.assertEqual(warns[0].subject, "L1-L7:stubbed")

    def test_a_mixed_seam_with_one_marked_side_does_not_warn(self) -> None:
        text = (LINT_FIXTURES / "clean" / "seams.toml").read_text().replace(
            'b = { layer = 7, files = [] }', 'b = { layer = 7, files = ["fx/built-side.rs"] }')
        m = compose_map(self.tmp)
        (m / "seams.toml").write_text(text)
        make_tree(self.tree, {"fx/built-side.rs": "//! Layer 7: built, no marker.\n"})
        self.assertEqual(lint_dir(m, self.tree).warns("stub-marker"), [], "side a still carries T0")
        make_tree(self.tree, {"fx/stub.rs": "//! Layer 1: no marker either.\n"})
        warns = lint_dir(m, self.tree).warns("stub-marker")
        self.assertEqual(len(warns), 1, "no side carries the marker: one warning for the seam")
        self.assertIn("fx/stub.rs", warns[0].message)
        self.assertIn("fx/built-side.rs", warns[0].message)

    def test_a_stub_with_no_side_file_on_disk_does_not_warn(self) -> None:
        (self.tree / "fx/stub.rs").unlink()
        self.assertEqual(self.lint().warns("stub-marker"), [])

    def test_marker_deep_in_the_file_does_not_count(self) -> None:
        make_tree(self.tree, {"fx/stub.rs": "//! x\n" * 30 + "//! T0 stub\n"})
        self.assertEqual(len(self.lint().warns("stub-marker")), 1)

    def test_planned_seam_whose_file_appeared_warns(self) -> None:
        make_tree(self.tree, {"fx/future-a.rs": "//! T0 stub\n"})
        result = self.lint()
        self.assertEqual(result.fails(), [])
        self.assertEqual([w.subject for w in result.warns("stub-marker")], ["L8-L9:later"])

    def test_no_fakes_no_parity_warning(self) -> None:
        text = (LINT_FIXTURES / "clean" / "checks.toml").read_text().replace('parity = "none"', 'parity = "l1.alpha"')
        m = compose_map(self.tmp)
        (m / "checks.toml").write_text(text)
        self.assertEqual(lint_dir(m, self.tree).warns("fake-parity"), [])

    def test_a_planned_seam_is_not_held_to_the_tree(self) -> None:
        # clean has a planned seam whose file and spec do not exist: no failure
        self.assertEqual(self.lint().fails("files"), [])


class Spec(unittest.TestCase):
    def test_section_numbers_match_standalone_tokens(self) -> None:
        for heading, number, want in [
            ("1. Intro", "1", True), ("2.1 Part", "2.1", True), ("2.1.3 Deeper", "2.1", True),
            ("14.2 Other", "4.2", False), ("4.25 Other", "4.2", False), ("Layer 3 notes", "3", True),
            ("10.1 Sub", "10", True), ("110 Big", "10", False), ("§10 Dashboard", "10", True),
            ("3.10 Tenth", "10", False), ("A1 Amendment", "A1", True),
        ]:
            self.assertEqual(section_matches(heading, number), want, f"{heading!r} vs §{number}")

    REAL_LINES = (
        "**7.2 Signed capability: role bits.** `appdata` v1 is `[1][roles u16 LE][0]` plus an optional 8 B hint.\n"
        "**9.4 The KB** stays inside the release payload at `current/share/kb/`; a KB change is a calver respin.\n"
        "**10.1 Rules (normative).** An own SVG radar in the lazy `nearby` chunk, replacing the ASCII placeholder\n"
        "**Binds:** `:11180` always, plus `:80` best-effort (`CAP_NET_BIND_SERVICE` from the unit).\n"
        "- **Probation (the actor that survives the restart):** the journal is written by root BEFORE the flip.\n"
        "- **Offload** is unchanged in v1: it stays on the LAN binding + pairing auth.\n"
        "- **Offload ← Layers 5/7.** A model-less node's agent points its model base URL at a nearby hub.\n"
    )

    def problems(self, ref: str, text: str | None = None) -> list[str]:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "s.md").write_text(self.REAL_LINES if text is None else text)
            return lint._spec_problems(ref, root, {})

    def test_bold_leads_are_section_anchors(self) -> None:
        for ref in ("s.md §7.2", "s.md §9.4", "s.md §10.1", "s.md §9.4, §10.1", "s.md §10"):
            self.assertEqual(self.problems(ref), [], ref)

    def test_bold_leads_are_matched_by_name(self) -> None:
        for ref in ("s.md Binds", "s.md Probation", "s.md 'The KB'", "s.md Offload", "s.md Signed capability",
                    "s.md Probation (the actor that survives the restart)"):
            self.assertEqual(self.problems(ref), [], ref)
        self.assertEqual(len(self.problems("s.md Nothing of the kind")), 1)

    def test_a_number_inside_a_lead_or_a_sentence_is_not_an_anchor(self) -> None:
        self.assertEqual(len(self.problems("s.md §5")), 1, "'Offload <- Layers 5/7' is not section 5")
        self.assertEqual(len(self.problems("s.md §9.5")), 1)
        self.assertEqual(len(self.problems("s.md §19.4")), 1)
        self.assertEqual(len(self.problems("s.md §4", "see **9.4 The KB** and 4.1 in prose\n")), 1)

    def test_a_bold_lead_in_a_code_fence_or_mid_line_is_not_an_anchor(self) -> None:
        text = "```\n**9.9 Fenced**\n```\nsome text **9.8 Mid-line** here\n"
        self.assertEqual(len(self.problems("s.md §9.9", text)), 1)
        self.assertEqual(len(self.problems("s.md §9.8", text)), 1)

    def test_a_lead_without_a_closing_mark_ends_at_its_colon(self) -> None:
        self.assertEqual(self.problems("s.md Health", "**Health: the checklist\n"), [])
        self.assertEqual(len(self.problems("s.md checklist", "**Health: the checklist\n")), 1)

    def test_the_numbered_heading_still_wins(self) -> None:
        self.assertEqual(self.problems("s.md §2", "### 2. Two\n"), [])

    def test_split_spec(self) -> None:
        self.assertEqual(split_spec("a/b.md §4.2"), ("a/b.md", "§4.2"))
        self.assertEqual(split_spec("a/b.md"), ("a/b.md", ""))
        self.assertEqual(split_spec("a/b.md Updates & rollback"), ("a/b.md", "Updates & rollback"))

    def test_quoted_heading_text_and_several_sections(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "s.md").write_text("## 3. Three\n\n## 19. Nineteen\n\n## Updates & rollback\n")
            cache: dict[str, list[str]] = {}
            self.assertEqual(lint._spec_problems("s.md §3, §19", root, cache), [])
            self.assertEqual(lint._spec_problems("s.md 'updates  &  rollback'", root, cache), [])
            self.assertEqual(len(lint._spec_problems("s.md §3, §20", root, cache)), 1)
            self.assertEqual(lint._spec_problems("nope.md §1", root, cache), ["spec file nope.md is missing"])


class Output(Base):
    def test_records_clean_one_pass_per_rule(self) -> None:
        recs = records(self.lint())
        self.assertEqual([r.id for r in recs], [f"x.seams.lint/{r}" for r, _ in RULES])
        self.assertTrue(all(r.status == "PASS" and r.check == "x.seams.lint" for r in recs))

    def test_records_fail_with_the_findings_words(self) -> None:
        recs = records(self.lint("files-side"))
        fails = [r for r in recs if r.status == "FAIL"]
        self.assertEqual(len(fails), 1)
        self.assertEqual(fails[0].id, "x.seams.lint/files/l3-l4-faked")
        self.assertIn("fx/no-such-file.txt", fails[0].assertion or "")
        self.assertEqual({r.id for r in recs if r.status == "PASS"},
                         {f"x.seams.lint/{r}" for r, _ in RULES if r != "files"})

    def test_two_findings_of_one_subject_get_distinct_ids(self) -> None:
        recs = records(self.lint("fake-mirror"))
        ids = [r.id for r in recs if r.status == "FAIL"]
        self.assertEqual(len(ids), len(set(ids)))
        self.assertEqual(len(ids), 2)

    def test_records_of_an_unloadable_map(self) -> None:
        m = compose_map(self.tmp)
        (m / "seams.toml").write_text("[[seam]]\n")
        recs = records(lint_dir(m, self.tree))
        self.assertTrue(recs and all(r.status == "FAIL" and "/schema/" in r.id for r in recs))

    def test_text_is_the_harness_idiom(self) -> None:
        text = render(self.lint())
        lines = text.splitlines()
        self.assertEqual(lines[0], "== 1. ids")
        self.assertTrue(lines[1].startswith("ok: ids: "), lines[1])
        self.assertEqual([ln for ln in lines if ln.startswith("== ")][-1], "== 8. known")
        self.assertIn("note: warn fake-parity: [[fake]]: 1 of 1 fake(s)", text)
        self.assertEqual(lines[-1], "LINT GREEN")
        self.assertFalse(any(ln.startswith("FAIL:") for ln in lines))

    def test_text_of_a_red_map(self) -> None:
        text = render(self.lint("unknown-seam"))
        self.assertIn("FAIL: unknown-seam: l4.api: seams names 'L9-L9:ghost'", text)
        self.assertEqual(text.splitlines()[-1], "LINT: 1 FAIL")

    def test_text_is_deterministic(self) -> None:
        self.assertEqual(render(self.lint("fake-mirror")), render(self.lint("fake-mirror")))

    def test_main_exit_codes(self) -> None:
        ns = argparse.Namespace()
        buf = io.StringIO()
        with redirect_stdout(buf):
            self.assertEqual(lint.main(ns, self.tree, compose_map(self.tmp)), 0)
            self.assertEqual(lint.main(ns, self.tree, compose_map(self.tmp, "unknown-seam")), 1)
        self.assertIn("LINT GREEN", buf.getvalue())
        self.assertIn("LINT: 1 FAIL", buf.getvalue())

    def run_cli(self, map_dir: Path) -> subprocess.CompletedProcess:
        return subprocess.run([sys.executable, "-I", str(RUN_PY), "--lint", "--root", str(self.tree), "--map",
                               str(map_dir)], cwd=self.tmp, text=True, capture_output=True, timeout=60,
                              start_new_session=True, check=False)

    def test_run_py_lint_end_to_end(self) -> None:
        green = self.run_cli(compose_map(self.tmp))
        self.assertEqual((green.returncode, green.stdout.splitlines()[-1]), (0, "LINT GREEN"), green.stderr)
        red = self.run_cli(compose_map(self.tmp, "files-side"))
        self.assertEqual(red.returncode, 1)
        self.assertEqual(red.stdout.splitlines()[-1], "LINT: 1 FAIL")
        self.assertIn("FAIL: files: L3-L4:faked: side a file fx/no-such-file.txt is missing", red.stdout)

    def test_lint_writes_nothing_into_the_tree(self) -> None:
        before = sorted(p.relative_to(self.tree).as_posix() for p in self.tree.rglob("*"))
        self.run_cli(compose_map(self.tmp))
        after = sorted(p.relative_to(self.tree).as_posix() for p in self.tree.rglob("*"))
        self.assertEqual(before, after)


if __name__ == "__main__":
    unittest.main()
