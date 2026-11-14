# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A landing's install after its turn (spec AC-104), on real repositories in a temporary folder; no remote is contacted."""
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_land  # noqa: E402
from test_land import git, land  # noqa: E402


class Install(test_land.Repos, unittest.TestCase):
    def installs(self, files, env="1"):
        with mock.patch.dict(os.environ, {"TM_LAND_INSTALL": env}):
            land.after_push(self.primary, files)
        return land.install_mark(self.primary).exists()

    def made(self, ran):
        def run(primary, target):
            ran.append(target)
            return subprocess.CompletedProcess([], 0, "terminumd: this checkout's build\n" if target == "installed" else "", "")
        return run

    def test_code_landed_marks_one_install_and_docs_none(self):  # AC-104
        self.assertFalse(self.installs({"docs/USAGE.md"}))
        self.assertFalse(self.installs({"terminumd/src/a.rs"}, env="0"))
        self.assertTrue(self.installs({"terminumd/src/a.rs"}))

    def test_the_install_runs_once_after_the_turn_for_all_that_landed(self):  # AC-104
        self.installs({"one.rs"})
        self.installs({"two.rs"})
        ran, said = [], []
        land.install_pending(self.primary, run=self.made(ran), say=said.append)
        land.install_pending(self.primary, run=self.made(ran), say=said.append)
        self.assertEqual(ran, ["build", "install", "installed"], "once for both")
        self.assertIn("make installed: exit 0: terminumd: this checkout's build", said)
        self.assertFalse(land.install_mark(self.primary).exists())

    def test_a_landing_in_the_turn_is_left_the_install(self):  # AC-104
        self.installs({"one.rs"})
        ran, said = [], []
        with land.Turn(self.tree("two")):
            land.install_pending(self.primary, run=self.made(ran), say=said.append)
        self.assertEqual((ran, said), ([], ["install: left to two's landing, which has the turn"]))
        self.assertTrue(land.install_mark(self.primary).exists(), "kept for two, which installs after its own")
        land.install_pending(self.primary, run=self.made(ran), say=said.append)
        self.assertEqual(ran, ["build", "install", "installed"])

    def test_a_failed_build_stops_the_install_and_is_said(self):  # AC-104
        self.installs({"one.rs"})
        said = []
        failed = lambda primary, target: subprocess.CompletedProcess([], 2, "", "error[E0425]")
        with mock.patch("sys.stderr"):
            land.install_pending(self.primary, run=failed, say=said.append)
        self.assertEqual(said, ["make build: exit 2", "install: the next landing tries again"])
        self.assertTrue(land.install_mark(self.primary).exists(), "kept for the next landing")
        ran = []
        land.install_pending(self.primary, run=self.made(ran), say=said.append)
        self.assertEqual(ran, ["build", "install", "installed"])

    def test_no_install_while_the_primary_checkout_is_not_on_what_landed(self):  # AC-104
        land.install_mark(self.primary).write_text(self.push_from_outside("c.txt", "x\n"))
        git("fetch", "-q", "origin", cwd=self.primary)
        ran, said = [], []
        land.install_pending(self.primary, run=self.made(ran), say=said.append)
        self.assertEqual((ran, said), ([], ["install: none, the primary checkout is not on what landed: the next landing tries again"]))
        self.assertTrue(land.install_mark(self.primary).exists())

    def test_a_mark_written_during_an_install_stays_for_its_landing(self):  # AC-104
        self.installs({"one.rs"})
        def run(primary, target):
            if target == "build":
                land.install_mark(primary).write_text("a later landing's\n" + land.install_mark(primary).read_text())
            return subprocess.CompletedProcess([], 0, "", "")
        land.install_pending(self.primary, run=run, say=lambda _: None)
        self.assertTrue(land.install_mark(self.primary).exists())


if __name__ == "__main__":
    unittest.main()
