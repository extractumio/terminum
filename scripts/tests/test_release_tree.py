# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""scripts/release_tree.py (spec AC-200): what a release pins, what it refuses, and the kept tree it runs in; on real
repositories in a temporary folder (a local origin, nothing contacted)."""
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_tree  # noqa: E402
from test_gate_record import git, Repo  # noqa: E402
import worktree  # noqa: E402


class Pin(Repo, unittest.TestCase):
    def setUp(self):
        self.make_repo()
        self.old = self.commit("old.txt")   # before releases in a tree of their own
        self.passed = self.commit("scripts/check_all.py")
        self.said = []

    def pin(self, wanted=None, rehearsing=False):
        return release_tree.pin(self.repo, wanted, rehearsing, self.said.append)

    def test_no_full_pass_pins_origin_main(self):  # AC-200
        self.assertEqual(self.pin(), self.passed)
        self.assertIn("no commit of it passed the full run in 48 hours", self.said[-1])

    def test_a_docs_only_successor_of_a_full_pass_is_pinned(self):  # AC-200
        self.pass_whole(self.passed)
        notes = self.commit("docs/releases/v1.0.0.md")
        self.assertEqual(self.pin(), notes)
        self.assertIn(f"release: pinned {notes[:9]}: docs only since {self.passed[:9]}'s full pass", self.said[-1])
        self.assertIn(f"1 commits since {self.passed[:9]}'s full pass", self.said[-1])

    def test_code_after_the_pass_keeps_the_pass_pinned(self):  # AC-200
        self.pass_whole(self.passed)
        self.commit("scripts/land.py")
        self.commit("README.md")
        self.assertEqual(self.pin(), self.passed)
        self.assertIn("2 commits behind origin/main", self.said[-1])
        self.assertNotEqual(self.pin(rehearsing=True), self.passed, "a rehearsal pins origin/main")

    def test_an_old_pass_is_no_pin(self):  # AC-200
        self.pass_whole(self.passed, at="2026-01-01 00:00")
        head = self.commit("docs/x.md")
        self.assertEqual(self.pin(), head)
        self.assertIn("no commit of it passed", self.said[-1])

    def test_pins_it_refuses(self):  # AC-200
        local = self.commit("scripts/fix.py", push=False)
        with self.assertRaisesRegex(worktree.Refused, f"{local[:9]} is not on origin/main: land it first"):
            self.pin(local)
        with self.assertRaisesRegex(worktree.Refused, "PIN=nope is no commit here"):
            self.pin("nope")
        with self.assertRaisesRegex(worktree.Refused, "predates releases in a tree of their own"):
            self.pin(self.old)
        self.assertEqual(self.pin(self.passed), self.passed)
        self.assertIn(f"PIN={self.passed}", self.said[-1])


class Tree(Repo, unittest.TestCase):
    def setUp(self):
        self.make_repo()
        self.first = self.commit("scripts/check_all.py")
        self.second = self.commit("b.txt")

    def head(self, path):
        return git("rev-parse", "HEAD", cwd=path)

    def test_the_kept_tree_is_made_detached_and_moved_to_each_pin(self):  # AC-200
        tree = release_tree.take(self.repo, "release", self.first)
        self.assertEqual((tree, self.head(tree)), (self.repo / ".worktrees" / "release", self.first))
        self.assertEqual(worktree.trees(self.repo)[tree.resolve()], None, "detached")
        self.assertEqual(self.head(release_tree.take(self.repo, "release", self.second)), self.second)
        self.assertEqual(worktree.sweep(self.repo), [], "a sweep never takes it")
        self.assertTrue(tree.is_dir())

    def test_a_kept_tree_with_uncommitted_changes_is_refused(self):  # AC-200
        tree = release_tree.take(self.repo, "release", self.first)
        (tree / "b.txt").write_text("mine\n")
        with self.assertRaisesRegex(worktree.Refused, "holds uncommitted changes, nothing of a release's own"):
            release_tree.take(self.repo, "release", self.second)

    def test_one_run_per_tree(self):  # AC-200
        with release_tree.Held(self.repo, "release") as one, release_tree.Held(self.repo, "release") as two, \
                release_tree.Held(self.repo, "rehearse") as other:
            self.assertTrue(one.take())
            one.hold_as("release v1.0.0 at 4f2a91c07, pid 4242")
            self.assertFalse(two.take())
            self.assertEqual(two.holder(), "release v1.0.0 at 4f2a91c07, pid 4242")
            self.assertTrue(other.take(), "a rehearsal has its own tree")

    def test_a_folder_in_the_way_is_refused(self):  # AC-200
        (self.repo / ".worktrees" / "rehearse").mkdir(parents=True)
        with self.assertRaisesRegex(worktree.Refused, "exists and is no worktree"):
            release_tree.take(self.repo, "rehearse", self.first)
        self.assertEqual(subprocess.run(["git", "status", "--porcelain"], cwd=self.repo, capture_output=True, text=True).stdout, "")


if __name__ == "__main__":
    unittest.main()
