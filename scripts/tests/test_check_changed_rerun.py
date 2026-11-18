# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A red Verify's failed steps, named for terminum, and run again on the target (spec AC-198); nothing real is run."""
from contextlib import redirect_stdout
import importlib.util
import io
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("check_changed", Path(__file__).resolve().parents[1] / "check_changed.py")
check_changed = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_changed)
from testrun import short_tmp
from test_land import git


class Rerun(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(dir=short_tmp())
        self.root = Path(self.tmp.name)
        git("init", "-q", "-b", "main", cwd=self.root)
        (self.root / "terminumd/src/foo").mkdir(parents=True)
        (self.root / "terminumd/src/foo/bar.rs").write_text("fn a() {}\n")
        git("add", ".", cwd=self.root)
        git("commit", "-qm", "base", cwd=self.root)
        self.base = git("rev-parse", "HEAD", cwd=self.root)
        # the task: one file the target has, one it added
        (self.root / "terminumd/src/foo/bar.rs").write_text("fn a() { b() }\n")
        (self.root / "terminumd/src/foo/new.rs").write_text("fn n() {}\n")
        git("add", ".", cwd=self.root)
        git("commit", "-qm", "task", cwd=self.root)
        self.task = git("rev-parse", "HEAD", cwd=self.root)
        git("checkout", "-q", "--detach", self.base, cwd=self.root)   # the target's tree
        (self.root / ".checks").mkdir()

    def tearDown(self):
        self.tmp.cleanup()

    def names(self, wanted):
        return [name for name, _ in check_changed.rerun_plan(wanted, self.task, self.root)]

    def test_only_the_named_steps_of_the_commits_plan_with_the_builds_they_run_on(self):  # AC-198
        self.assertEqual(self.names({"cargo"}), ["ui", "debug-build", "cargo"])
        cargo = dict(check_changed.rerun_plan({"cargo"}, self.task, self.root))["cargo"]
        self.assertIn("foo::bar", cargo)
        self.assertNotIn("foo::new", cargo, "a file the target lacks makes nothing of the target's run")
        self.assertEqual(self.names({"e2e_new", "security"}), ["security"], "the scan needs no build")

    def test_a_step_the_targets_plan_lacks_runs_nothing(self):  # AC-198
        self.assertEqual(self.names({"e2e_new"}), [], "not even the builds")

    def test_module_filters_are_quoted(self):  # AC-198
        self.assertTrue(check_changed.cargo_test({"a::b", "x; id"}).endswith("-- a::b 'x; id'"))

    def rerun(self, failed, skipped=False):
        def run(steps, root, **_):
            if skipped:
                (root / ".checks" / "skipped").write_text("targeted: SKIPPED timed checks, the machine busy for 15 minutes: critical\n")
            return failed
        with mock.patch.object(check_changed, "run", run), redirect_stdout(io.StringIO()) as said:
            code = check_changed.rerun({"cargo"}, self.task, self.root)
        note = self.root / ".checks" / "failed"
        return code, note.read_text() if note.exists() else None, said.getvalue()

    def test_red_on_the_target_names_its_red_steps(self):  # AC-198
        code, note, said = self.rerun(["cargo"])
        self.assertEqual((code, note), (1, "cargo\n"))
        self.assertIn("rerun: FAIL cargo", said)

    def test_green_on_the_target_names_none(self):  # AC-198
        (self.root / ".checks" / "failed").write_text("cargo\n")
        self.assertEqual(self.rerun([])[:2], (0, None))

    def test_a_step_skipped_for_a_busy_machine_is_not_known(self):  # AC-146, AC-198
        code, note, said = self.rerun([], skipped=True)
        self.assertEqual((code, note), (1, None))
        self.assertIn("rerun: SKIPPED, not known", said)

    def test_a_red_targeted_run_names_its_failed_steps_and_a_green_one_none(self):  # AC-198
        check_changed.say_failed(self.root, ["critical", "page"])
        self.assertEqual((self.root / ".checks" / "failed").read_text(), "critical\npage\n")
        check_changed.say_failed(self.root, [])
        self.assertFalse((self.root / ".checks" / "failed").exists())

    def test_rerun_needs_its_commit(self):  # AC-198
        with redirect_stdout(io.StringIO()), mock.patch.object(sys, "stderr", io.StringIO()), self.assertRaises(SystemExit):
            check_changed.main(["--rerun", "cargo"])


if __name__ == "__main__":
    unittest.main()
