# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-198: concurrent landings and After share a checkout; unknown queue state never publishes.

Real private Git repositories and locks; build/install commands are stand-ins, never the live install.
"""
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path
import os
import subprocess
import sys
import threading
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import test_land  # noqa: E402
import land_stages as stages  # noqa: E402
L = stages.land


class AfterRace(test_land.Repos, unittest.TestCase):
    def test_second_landing_publishes_during_after_but_waits_to_move_the_checkout(self):
        second = self.tree("second")
        second_commit = self.commit(second, "second.rs", "second\n")
        first_commit = L.head(self.primary)
        ran, built, installed = [], [], []
        moving = threading.Event()

        def say(*args, **_):
            if "waiting before moving the checkout" in " ".join(map(str, args)):
                moving.set()

        def after_push(files):
            L.after_push(self.primary, files)

        def second_landing():
            return L.land(second, check=lambda _: True, passed=lambda _: True,
                          smoke=lambda _: True, after=after_push, say=lambda _: None)

        def run(primary, target):
            ran.append(target)
            if target == "build":
                built.append(L.head(primary))
                if len(built) == 1:
                    future.append(pool.submit(second_landing))
                    self.assertTrue(moving.wait(10), "second landing waits for the install lock")
                    self.assertEqual(self.on_origin(), second_commit)
                    self.assertEqual(L.head(primary), first_commit, "checkout stayed on the first build")
                    self.assertEqual(L.install_mark(primary).read_text(), first_commit)
            if target == "install":
                installed[:] = [built[-1]]
            code = int(target == "installed" and installed != [L.head(primary)])
            return subprocess.CompletedProcess([], code, "installed: verified\n", "")

        future = []
        with mock.patch("builtins.print", say), ThreadPoolExecutor(max_workers=1) as pool:
            stages.after(self.primary, run=run, env={"TM_LAND_INSTALL": "1"}, say=say)
            self.assertEqual(future[0].result(timeout=10), second_commit)
        self.assertEqual(installed, [first_commit], "first After ended green before the primary moved")
        self.assertEqual(L.head(self.primary), second_commit)
        stages.after(self.primary, run=run, env={"TM_LAND_INSTALL": "1"}, say=say)
        self.assertEqual(installed, [second_commit], "next After went on and installed its build")
        self.assertEqual(ran, ["installed", "build", "install", "installed"] * 2)
        self.assertFalse(L.install_mark(self.primary).exists())

    def test_after_retries_a_forward_move_and_keeps_a_later_landings_mark(self):
        first = L.head(self.primary)
        second = self.push_from_outside("second.rs", "second\n")
        ran, built, installed, said = [], [], [], []

        def run(primary, target):
            ran.append(target)
            if target == "build":
                built.append(L.head(primary))
            if target == "install":
                installed[:] = [built[-1]]
                if L.head(primary) == first:
                    # An older landing ignores the install lock and moves main during this install.
                    L.git("fetch", "-q", "origin", cwd=primary)
                    self.assertIsNone(L.update_primary(primary))
                    L.install_mark(primary).write_text(second)
            code = int(target == "installed" and installed != [L.head(primary)])
            return subprocess.CompletedProcess([], code, "installed: checked\n", "")

        stages.after(self.primary, run=run, env={"TM_LAND_INSTALL": "1"}, say=said.append)
        self.assertEqual(ran, ["installed", "build", "install", "installed", "build", "install", "installed"])
        self.assertEqual(installed, [second])
        self.assertTrue(any("retrying once" in line for line in said))
        self.assertEqual(L.install_mark(self.primary).read_text(), second, "the later landing owns its mark")

    def test_install_freshen_can_advance_to_a_second_publish_without_failing_after(self):
        import worktree_fresh
        first = L.head(self.primary)
        installed, built, published, ran = [], [], [], []

        def run(primary, target):
            ran.append(target)
            if target == "build":
                built.append(L.head(primary))
                published.append(self.push_from_outside("second.rs", "second\n"))
            if target == "install":
                # make install freshens then invokes a new make install-stage, with a fresh build ID.
                worktree_fresh.freshen(primary)
                installed[:] = [L.head(primary)]
            code = int(target == "installed" and installed != [L.head(primary)])
            return subprocess.CompletedProcess([], code, "installed: checked\n", "")

        stages.after(self.primary, run=run, env={"TM_LAND_INSTALL": "1"}, say=lambda _: None)
        self.assertEqual((built, installed), ([first], published))
        self.assertEqual(ran, ["installed", "build", "install", "installed"], "freshen needs no retry when verification passes")
        self.assertFalse(L.install_mark(self.primary).exists())

    def test_only_a_clean_forward_move_with_failed_verification_gets_one_retry(self):
        for failure in ("build", "install", "stable", "dirty", "backward", "diverged", "branch", "twice"):
            with self.subTest(failure=failure):
                # A fresh private repository for each independent failure path.
                case = test_land.Repos()
                case.addCleanup = self.addCleanup
                case.setUp()
                before = L.head(case.primary)
                old = L.git("rev-parse", "HEAD^", cwd=case.primary).stdout.strip()
                ran = []

                def run(primary, target):
                    ran.append(target)
                    if target == "install" and failure != "stable":
                        case.commit(primary, f"advance-{len(ran)}.rs", "advance\n")
                        if failure == "dirty":
                            (primary / "loose.txt").write_text("uncommitted\n")
                        if failure == "backward":
                            L.git("checkout", "--detach", old, cwd=primary)
                        if failure == "diverged":
                            sibling = L.git("commit-tree", "HEAD^{tree}", "-p", old, "-m", "sibling", cwd=primary).stdout.strip()
                            L.git("update-ref", "refs/heads/main", sibling, cwd=primary)
                            self.assertIsNone(L.dirty_words(primary), "same branch and clean, but not a descendant")
                        if failure == "branch":
                            L.git("checkout", "-b", "another", cwd=primary)
                    code = int(target == failure or target == "installed")
                    return subprocess.CompletedProcess([], code, "mismatch\n", "")

                with self.assertRaises(stages.Stop) as stop:
                    stages.after(case.primary, run=run, env={"TM_LAND_INSTALL": "1"}, say=lambda _: None)
                self.assertEqual(stop.exception.code, 1, "real install failures keep After red")
                self.assertTrue(L.install_mark(case.primary).exists())
                self.assertEqual(ran.count("build"), 2 if failure == "twice" else 1)
                if failure == "stable":
                    self.assertEqual(L.head(case.primary), before)

    def test_unknown_integration_status_refuses_land_and_manual_stages_without_publication(self):
        tree = self.tree("blocked")
        self.commit(tree, "blocked.rs", "x\n")
        before = self.on_origin()
        # These are manual calls even when the test itself runs inside queue Verify.
        with mock.patch.dict(os.environ, {"TM_INTEGRATION_TASK": "", "TM_INTEGRATION_BATCH": ""}), \
                mock.patch.object(L, "primary_of", return_value=self.primary), \
                mock.patch.object(stages, "integration_on", side_effect=stages.Stop(1, "integration not known: nothing pushed")), \
                mock.patch.object(L, "land") as local, mock.patch.object(L, "publish") as publish, \
                mock.patch.object(stages, "after") as after, mock.patch("sys.stderr"):
            self.assertEqual(L.main(["blocked"]), 1)
            self.assertEqual(L.main(["publish", "blocked"]), 1)
            self.assertEqual(L.main(["after"]), 1)
            local.assert_not_called()
            publish.assert_not_called()
            after.assert_not_called()
        self.assertEqual(self.on_origin(), before)


if __name__ == "__main__":
    unittest.main()
