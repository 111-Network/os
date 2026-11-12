"""``--changed [REV]``: a temp git repo, changed paths mapped through side files and check files to seams and units."""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from t111 import changed
from t111.changed import GitError, analyse, changed_paths, seams_for_path
from t111.schema import load_map

from .helpers import RUN_PY
from .test_lint import compose_map, make_tree

GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", "-c", "commit.gpgsign=false"]


class Repo(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = Path(tempfile.mkdtemp(prefix="t111-changed-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.tree = make_tree(self.tmp / "tree")
        shutil.copytree(compose_map(self.tmp), self.tree / "verify")  # the map lives in the tree, as in the repo
        self.map_dir = self.tree / "verify"
        self.git("init", "-q", ".")
        self.commit("base")
        self.map = load_map(self.map_dir)

    def git(self, *args: str) -> str:
        proc = subprocess.run(["git", *args], cwd=self.tree, text=True, capture_output=True, timeout=60,
                              start_new_session=True, check=True)
        return proc.stdout

    def commit(self, msg: str) -> None:
        self.git("add", "-A")
        subprocess.run(GIT + ["commit", "-q", "-m", msg], cwd=self.tree, check=True, capture_output=True, timeout=60,
                       start_new_session=True)

    def write(self, rel: str, text: str) -> None:
        path = self.tree / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def run_changed(self, rev: str | None = None, scratch: str | None = None) -> subprocess.CompletedProcess:
        argv = [sys.executable, "-I", str(RUN_PY), "--root", str(self.tree), "--map", str(self.map_dir),
                "--changed" if rev is None else f"--changed={rev}"]
        if scratch:
            argv += ["--scratch", scratch]
        return subprocess.run(argv, cwd=self.tmp, text=True, capture_output=True, timeout=60, start_new_session=True,
                              check=False)


class Mapping(Repo):
    def test_side_files_map_to_seams_exact_directory_and_glob(self) -> None:
        got = lambda p: [(s.id, side) for s, side in seams_for_path(self.map, p)]  # noqa: E731
        self.assertEqual(got("fx/c.txt"), [("L3-L4:faked", "a")])
        self.assertEqual(got("fx/a.txt"), [("L1-L2:alpha", "a"), ("X:notices", "a")])
        self.assertEqual(got("fx/gen/deep/x.txt"), [("X:notices", "b")], "a side file may be a directory")
        self.assertEqual(got("fx/other.txt"), [])

    def test_a_glob_entry_matches(self) -> None:
        from dataclasses import replace
        seam = self.map.seam("L1-L2:alpha")
        assert seam is not None
        globbed = replace(seam, a=replace(seam.a, files=("fx/*.py",)))
        smap = replace(self.map, seams=(globbed,))
        self.assertEqual([s.id for s, _ in seams_for_path(smap, "fx/check.py")], ["L1-L2:alpha"])

    def test_check_files_map_to_checks_and_their_seams(self) -> None:
        paths = ["1-bootstrap/crates/node111/tests/api_x.rs", "4-interface/ui/test/a.test.ts", "fx/check.py", "README.md"]
        found = analyse(self.map, self.tree, paths, [])
        self.assertEqual(found["check_paths"]["l4.api"], ["1-bootstrap/crates/node111/tests/api_x.rs"])
        self.assertEqual(found["check_paths"]["l4.ui.a"], ["4-interface/ui/test/a.test.ts"])
        self.assertEqual(set(found["check_paths"]["l1.alpha"]), {"fx/check.py"})
        self.assertIn("L3-L4:faked", found["seams"])
        self.assertIn("L1-L2:alpha", found["seams"], "through the checks that claim fx/check.py")
        self.assertEqual(found["unmapped"], ["README.md"])

    def test_map_and_runner_paths_are_not_unmapped(self) -> None:
        found = analyse(self.map, self.tree, ["verify/seams.toml", "verify/t111/x.py", "verify/tests/t.py"],
                        ["verify/seams.toml"])
        self.assertEqual(found["unmapped"], [])
        self.assertEqual(len(found["map_touched"]), 3)


class Git(Repo):
    def test_nothing_changed(self) -> None:
        proc = self.run_changed()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("changed against HEAD: 0 path(s)", proc.stdout)
        self.assertIn("nothing to run", proc.stdout)

    def test_tracked_and_untracked_paths(self) -> None:
        self.write("fx/c.txt", "changed\n")
        self.write("fx/new.txt", "new\n")
        tracked, untracked = changed_paths(self.tree, "HEAD")
        self.assertEqual((tracked, untracked), (["fx/c.txt"], ["fx/new.txt"]))

    def test_report_for_a_working_tree_change(self) -> None:
        self.write("fx/c.txt", "changed\n")
        self.write("4-interface/ui/test/a.test.ts", "// edited\n")
        self.write("docs/note.md", "new, untracked\n")
        proc = self.run_changed()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        out = proc.stdout
        self.assertIn("changed against HEAD: 3 path(s) (2 tracked, 1 untracked)", out)
        self.assertIn("L3-L4:faked [built] <- fx/c.txt (side a, L4)", out)
        self.assertIn("l4.ui.a <- 4-interface/ui/test/a.test.ts", out)
        self.assertIn("no seam and no check maps 1 path(s): docs/note.md", out)
        self.assertIn("python3 -I verify/run.py --scratch DIR --seam L3-L4:faked --check l4.ui.a", out)
        self.assertIn("checks: l4.api (slow), l4.ui.a (slow)", out)
        self.assertNotIn("--lint", out)

    def test_scratch_flag_is_echoed_in_the_command(self) -> None:
        self.write("fx/c.txt", "changed\n")
        proc = self.run_changed(scratch="/tmp/somewhere")
        self.assertIn("--scratch /tmp/somewhere --seam L3-L4:faked", proc.stdout)

    def test_against_an_older_revision(self) -> None:
        self.write("fx/a.txt", "second commit\n")
        self.commit("second")
        quiet = self.run_changed()
        self.assertIn("0 path(s)", quiet.stdout)
        proc = self.run_changed("HEAD~1")
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("changed against HEAD~1: 1 path(s) (1 tracked, 0 untracked)", proc.stdout)
        self.assertIn("L1-L2:alpha [built] <- fx/a.txt (side a, L1)", proc.stdout)
        self.assertIn("X:notices [built] <- fx/a.txt (side a, X)", proc.stdout)

    def test_a_range_ignores_untracked_files(self) -> None:
        self.write("fx/a.txt", "second\n")
        self.commit("second")
        self.write("fx/untracked.txt", "u\n")
        self.assertEqual(changed_paths(self.tree, "HEAD~1..HEAD"), (["fx/a.txt"], []))

    def test_map_change_points_at_lint_and_the_unit_tests(self) -> None:
        self.write("verify/seams.toml", (self.map_dir / "seams.toml").read_text() + "\n# touched\n")
        proc = self.run_changed()
        self.assertIn("the map or the runner changed", proc.stdout)
        self.assertIn("python3 -I verify/run.py --lint", proc.stdout)
        self.assertIn("unittest discover -s verify/tests", proc.stdout)
        self.assertIn("no seam or check to run for these paths", proc.stdout)

    def test_a_changed_planned_seam_file_names_the_gap(self) -> None:
        self.write("fx/future-a.rs", "//! new\n")
        self.write("fx/c.txt", "changed\n")
        proc = self.run_changed()
        self.assertIn("L8-L9:later [planned]", proc.stdout)
        self.assertIn("no check in these tiers proves: L8-L9:later", proc.stdout)

    def test_only_a_planned_seam_has_nothing_to_run(self) -> None:
        self.write("fx/future-a.rs", "//! new\n")
        proc = self.run_changed()
        self.assertEqual(proc.returncode, 0)
        self.assertIn("no check covers them in the default tiers", proc.stdout)

    def test_a_bad_revision_exits_2_and_a_dash_revision_is_refused(self) -> None:
        bad = self.run_changed("no-such-rev")
        self.assertEqual(bad.returncode, 2)
        self.assertIn("--changed:", bad.stderr)
        with self.assertRaises(GitError):
            changed_paths(self.tree, "--output=/tmp/x")
        with self.assertRaises(GitError):
            changed_paths(self.tree, "  ")

    def test_git_leaves_the_repo_alone(self) -> None:
        self.write("fx/c.txt", "changed\n")
        before = self.git("status", "--porcelain=v1")
        self.run_changed()
        self.assertEqual(self.git("status", "--porcelain=v1"), before)
        self.assertEqual(sorted(p.name for p in (self.tree / ".git").glob("*.lock")), [])

    def test_the_command_names_ns_checks_the_default_tiers_leave_out(self) -> None:
        from dataclasses import replace
        ui = self.map.check("l4.ui.a")
        assert ui is not None
        smap = replace(self.map, checks=tuple(replace(c, tier="ns") if c.id == "l4.ui.a" else c for c in self.map.checks))
        found = analyse(smap, self.tree, ["fx/c.txt"], [])
        text = changed.render(smap, found, "HEAD", 1, 0, "DIR")
        self.assertIn("also covering, in a tier not asked for (add --tier fast,slow,ns): l4.ui.a", text)


if __name__ == "__main__":
    unittest.main()
