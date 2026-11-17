# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""scripts/gate_record.py (spec AC-201, AC-202): passes per step and content shared by every tree of a repository,
their age, the newest full pass on a line of commits, the record of runs and what the nightly's agent reads from it.
On real repositories in a temporary folder; no remote is contacted."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gate_record  # noqa: E402


def git(*args, cwd):
    return subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *args], cwd=cwd, check=True,
                          capture_output=True, text=True).stdout.strip()


class Repo:
    """An origin and a clone with commits, for the tests here and of the release's tree."""
    def make_repo(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name).resolve()
        self.origin = self.root / "origin.git"
        git("init", "-q", "--bare", "-b", "main", str(self.origin), cwd=self.root)
        self.repo = self.root / "primary"
        git("clone", "-q", str(self.origin), str(self.repo), cwd=self.root)
        git("checkout", "-q", "-b", "main", cwd=self.repo)
        self.commit(".gitignore", ".checks/\n")   # as the repository's own

    def commit(self, name, text="x\n", push=True):
        path = self.repo / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        git("add", name, cwd=self.repo)
        git("commit", "-q", "-m", name, cwd=self.repo)
        if push:
            git("push", "-q", "origin", "main", cwd=self.repo)
        return git("rev-parse", "HEAD", cwd=self.repo)

    def pass_whole(self, commit, at=None):
        gate_record.keep(self.repo, gate_record.content_of(self.repo, commit), {})
        if at:   # an older pass
            for mark in gate_record.marks(self.repo, gate_record.content_of(self.repo, commit)):
                mark.write_text(json.dumps({"at": at}) + "\n")


class Passes(Repo, unittest.TestCase):
    def setUp(self):
        self.make_repo()
        self.first = self.commit("a.txt")

    def test_a_step_pass_is_found_by_another_tree_of_the_repository(self):  # AC-201
        ident = gate_record.content_of(self.repo, self.first)
        gate_record.keep(self.repo, ident, {"retried": ["page webBoardPanel"]}, "page")
        other = self.root / "other"
        git("worktree", "add", "-q", "--detach", str(other), self.first, cwd=self.repo)
        notes = gate_record.passed(other, ident, "page")
        self.assertEqual(notes["retried"], ["page webBoardPanel"], "its notes travel with it")
        self.assertIsNone(gate_record.passed(other, ident, "live"))
        gate_record.forget(self.repo, ident, "page")
        self.assertIsNone(gate_record.passed(other, ident, "page"))

    def test_a_pass_written_before_notes_reads_as_its_date(self):  # AC-201
        mark = self.repo / ".checks" / "0123456789abcdef.ok"
        mark.parent.mkdir()
        mark.write_text("2026-11-07 12:00\n")
        self.assertEqual(gate_record.passed(self.repo, "0123456789abcdef"), {"at": "2026-11-07 12:00"})

    def test_young_counts_hours(self):  # AC-201
        self.assertTrue(gate_record.young({"at": gate_record.now()}, 48))
        old = time.strftime("%Y-%m-%d %H:%M", time.localtime(time.time() - 49 * 3600))
        self.assertFalse(gate_record.young({"at": old}, 48))
        self.assertFalse(gate_record.young({}, 48))

    def test_the_newest_full_pass_on_a_line_of_commits(self):  # AC-200
        second, third = self.commit("b.txt"), self.commit("c.txt")
        self.assertIsNone(gate_record.newest_full_pass(self.repo))
        self.pass_whole(self.first)
        self.pass_whole(second, at="2026-01-01 00:00")
        self.assertEqual(gate_record.newest_full_pass(self.repo, "main")[0], second)
        self.assertEqual(gate_record.newest_full_pass(self.repo, "main", 48)[0], self.first, "an old pass skipped for a release")
        self.assertNotEqual(third, second)


class Record(Repo, unittest.TestCase):
    def setUp(self):
        self.make_repo()
        self.head = self.commit("a.txt")

    def run_with(self, steps, by="check", commit=None):
        gate_record.append(self.repo, {"kind": "gate", "by": by, "commit": commit, "content": "c", "steps": steps, "result": "x"})

    def test_lines_are_kept_in_git_common_folder_and_read_by_kind(self):  # AC-201
        self.run_with({"live": {"result": "pass"}})
        gate_record.append(self.repo, {"kind": "release", "tag": "v1.0.0"})
        self.assertTrue((Path(git("rev-parse", "--path-format=absolute", "--git-common-dir", cwd=self.repo)) / gate_record.RECORD).is_file())
        self.assertEqual([e["kind"] for e in gate_record.read(self.repo)], ["gate", "release"])
        self.assertEqual(len(gate_record.read(self.repo, "gate")), 1)
        self.assertTrue(all("at" in e for e in gate_record.read(self.repo)))

    def test_a_check_retried_in_3_of_the_last_7_runs_that_ran_it_loses_its_retry(self):  # AC-201
        retried = {"page": {"result": "pass", "retried": ["page webBoardPanel"]}}
        for steps in (retried, retried):
            self.run_with(steps)
        for _ in range(6):
            self.run_with({"live": {"result": "pass"}})   # runs without page do not dilute it
        lately = lambda: gate_record.retried_lately(gate_record.read(self.repo, "gate"), "page webBoardPanel")  # noqa: E731
        self.assertFalse(lately())
        self.run_with(retried)
        self.assertTrue(lately())
        for _ in range(5):
            self.run_with({"page": {"result": "pass"}})
        self.assertFalse(lately(), "two of the last seven")

    def test_the_nightly_report_names_each_red_step_and_the_commits_since_the_last_pass(self):  # AC-202
        self.assertEqual(gate_record.nightly(self.repo), ["nightly: no rehearsal's full run in the record"])
        self.pass_whole(self.head)
        landed = self.commit("b.txt")
        self.run_with({"live": {"result": "fail", "cause": "failed twice", "crash": "tmux's server gone"},
                       "page": {"result": "pass", "retried": ["page webBoardPanel"], "timing": ["TIMING over budget: x"]}},
                      by="rehearse", commit=landed)
        said = "\n".join(gate_record.nightly(self.repo))
        self.assertIn(f"TASK: Bug: the nightly full run is red at live on {landed[:9]} (P1): failed twice; crash: tmux's server gone", said)
        self.assertIn(git("log", "--oneline", "-1", cwd=self.repo), said, "the commits since the last full pass")
        self.assertIn("RETRIED: page webBoardPanel", said)
        self.assertIn("TIMING (page): TIMING over budget: x", said)


if __name__ == "__main__":
    unittest.main()
