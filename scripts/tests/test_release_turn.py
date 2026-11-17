# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""scripts/release_turn.py (spec AC-200, AC-104, AC-146): the chain of make release and make rehearse in the pinned
tree, its steps stood in for: what stops it, what is only said, what it keeps and records; and the outer step that pins
and takes the tree. The real steps have their own tests (test_check_all.py, test_release_tree.py, test_release.py)."""
import os
from pathlib import Path
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gate_record  # noqa: E402
import release_turn  # noqa: E402
from test_gate_record import Repo  # noqa: E402

REAL_RUN = subprocess.run   # git and the rest go through; the scripts of the chain are stood in for


class Chain(Repo, unittest.TestCase):
    def setUp(self):
        self.make_repo()
        self.head = self.commit("a.txt")
        self.calls, self.released, self.said = [], [], []
        self.gate, self.mac, self.mac_runs = 0, (False, ""), 0
        patches = [mock.patch.object(release_turn.subprocess, "run", self.fake_run),
                   mock.patch.object(release_turn, "mac_checks", self.mac_checks),
                   mock.patch.object(release_turn.release, "main", self.release)]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)

    def fake_run(self, cmd, **kwargs):
        if cmd[0] != sys.executable:
            return REAL_RUN(cmd, **kwargs)
        self.calls.append(cmd[1:])
        if cmd[1:2] == ["scripts/check_all.py"]:   # its own tests: test_check_all.py
            if self.gate == 75:
                return subprocess.CompletedProcess(cmd, 75)
            if self.gate == 0:
                gate_record.keep(self.repo, self.ident(), {"retried": ["page webBoardPanel"]})
            gate_record.append(self.repo, {"kind": "gate", "content": self.ident(),
                                           "steps": {"live": {"result": "fail" if self.gate else "pass", "cause": "failed twice"}}})
            return subprocess.CompletedProcess(cmd, self.gate)
        return subprocess.CompletedProcess(cmd, 0)   # leftovers

    def ident(self):
        return gate_record.content_of(self.repo, self.head)

    def mac_checks(self, repo):
        self.mac_runs += 1
        return self.mac

    def release(self, tag, head, said="", rehearsing=False, retried=()):
        self.released.append((tag, head, said, rehearsing, list(retried)))

    def chain(self, rehearsing=False):
        return release_turn.chain(self.repo, "v1.0.0", rehearsing, lambda *w, **_: self.said.append(" ".join(w)))

    def last(self):
        return gate_record.read(self.repo)[-1]

    def test_a_release_hands_the_pinned_commit_and_its_retried_checks_on(self):  # AC-200, AC-201
        self.assertEqual(self.chain(), 0)
        self.assertEqual(self.released, [("v1.0.0", self.head, "", False, ["page webBoardPanel"])])
        self.assertEqual(self.calls, [["scripts/leftovers.py", "--end"], ["scripts/check_all.py", "--by", "release"]])
        entry = self.last()
        self.assertEqual((entry["kind"], entry["commit"], entry["result"], entry["mac"]), ("release", self.head, "published", "pass"))

    def test_a_busy_build_server_ends_the_release_before_any_test(self):  # AC-146
        self.gate = 75
        self.assertEqual(self.chain(), 1)
        self.assertIn("release: the build server is not available for a release: for 15 minutes: nothing is public", self.said)
        self.assertEqual((self.released, self.last()["result"]), ([], "busy"))

    def test_failing_tests_here_or_on_the_mac_stop_it_and_are_recorded(self):  # AC-104, AC-201
        self.gate = 1
        self.assertEqual(self.chain(), 1)
        self.assertEqual((self.released, self.last()["red"]), ([], {"live": "failed twice"}))
        self.gate, self.mac = 0, (True, "")
        self.assertEqual(self.chain(), 1)
        self.assertEqual((self.released, self.last()["result"]), ([], "red: the Mac test runner"))

    def test_the_mac_runners_pass_is_kept_per_content(self):  # AC-200
        self.chain()
        self.chain()
        self.assertEqual(self.mac_runs, 1)
        self.assertIn("release: the Mac test runner's checks passed on this content already", self.said)

    def test_a_runner_that_could_not_run_them_is_said_and_stops_nothing(self):  # AC-104
        self.mac = (False, "the runner could not run them")
        self.assertEqual(self.chain(), 0)
        self.assertIn("did not run: the runner could not run them", self.released[0][2], "for the release's notes")
        self.assertIn("release: the Mac test runner's checks did not run: the runner could not run them", self.said[-1])
        self.assertEqual(self.mac_runs, 1)
        self.chain()
        self.assertEqual(self.mac_runs, 2, "no pass kept for checks that did not run")

    def test_a_rehearsal_runs_its_full_run_by_its_own_rules(self):  # AC-202
        self.assertEqual(self.chain(rehearsing=True), 0)
        self.assertIn(["scripts/check_all.py", "--by", "rehearse"], self.calls)
        self.assertEqual(self.released[0][3], True)
        self.assertEqual(self.last()["result"], "rehearsed")

    def test_a_refused_release_is_recorded_and_still_says_what_did_not_run(self):  # AC-104
        self.mac = (False, "the runner could not run them")
        with mock.patch.object(self, "release", side_effect=SystemExit("HEAD moved")), \
                mock.patch.object(release_turn.release, "main", side_effect=SystemExit("HEAD moved")), \
                self.assertRaisesRegex(SystemExit, "HEAD moved"):
            self.chain()
        self.assertIn("did not run", self.said[-1])
        self.assertEqual(self.last()["result"], "stopped: HEAD moved")


class Outer(unittest.TestCase):
    def test_the_chain_runs_only_in_its_pinned_tree(self):  # AC-200
        with mock.patch.object(release_turn, "primary_of", return_value=Path(release_turn.__file__).resolve().parent.parent), \
                self.assertRaisesRegex(SystemExit, "the chain runs in its pinned tree"):
            release_turn.main(["--here", "v1.0.0"])

    def test_settings_and_source_are_looked_at_before_anything_runs(self):  # AC-104
        with mock.patch.object(release_turn, "primary_of", return_value=Path("/elsewhere")), \
                mock.patch.object(release_turn.release, "check_settings", side_effect=SystemExit("no identity")), \
                mock.patch.object(release_turn, "chain") as chain, self.assertRaisesRegex(SystemExit, "no identity"):
            release_turn.main(["--here", "v1.0.0"])
        chain.assert_not_called()
        with mock.patch.object(release_turn, "primary_of", return_value=Path("/elsewhere")), \
                mock.patch.object(release_turn.release, "check_settings"), \
                mock.patch.object(release_turn.release, "check_source", side_effect=SystemExit("the tree has changes")), \
                mock.patch.object(release_turn, "chain") as chain, self.assertRaisesRegex(SystemExit, "the tree has changes"):
            release_turn.main(["--here", "v1.0.0"])
        chain.assert_not_called()

    def test_a_bad_version_is_refused_at_once(self):
        for bad in ("1.0", "v1.0", "v1.0.0.1"):
            with self.assertRaisesRegex(SystemExit, "usage"):
                release_turn.main([bad])

    def test_another_run_in_the_tree_is_refused(self):  # AC-200
        held = mock.MagicMock()
        held.__enter__.return_value = held
        held.take.return_value = False
        held.holder.return_value = "release v1.0.0 at 4f2a91c07, pid 4242"
        with mock.patch.object(release_turn.release_tree, "pin", return_value="4f2a91c07aa"), \
                mock.patch.object(release_turn.release_tree, "Held", return_value=held), \
                mock.patch.object(release_turn.release, "tag_exists", return_value=False), \
                mock.patch.object(release_turn.release_tree, "take") as take, \
                self.assertRaisesRegex(SystemExit, "release: another release runs in .worktrees/release: release v1.0.0 at 4f2a91c07, pid 4242"):
            release_turn.main(["v1.0.0"])
        take.assert_not_called()

    def test_the_pinned_trees_own_make_runs_the_rest(self):  # AC-200
        held = mock.MagicMock()
        held.__enter__.return_value = held
        held.take.return_value = True
        with mock.patch.object(release_turn, "primary_of", return_value=Path("/w")), \
                mock.patch.object(release_turn.release_tree, "pin", return_value="4f2a91c07aa") as pin, \
                mock.patch.object(release_turn.release_tree, "Held", return_value=held), \
                mock.patch.object(release_turn.release, "tag_exists", return_value=True), \
                mock.patch.object(release_turn.release_tree, "take", return_value=Path("/w/.worktrees/rehearse")), \
                mock.patch.object(release_turn.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)) as run, \
                mock.patch("builtins.print"):
            self.assertEqual(release_turn.main(["--rehearse", "v1.0.0"]), 0)
        self.assertEqual(pin.call_args.args[1:3], ("refs/tags/v1.0.0", True), "a tag that exists is its own pin")
        self.assertEqual(run.call_args.args[0], ["make", "--no-print-directory", "rehearse-here", "TAG=v1.0.0"])
        self.assertEqual(run.call_args.kwargs["cwd"], Path("/w/.worktrees/rehearse"))


class MacChecks(unittest.TestCase):
    def status(self, code):
        run = mock.Mock()
        run.wait.return_value = code
        with mock.patch.dict(os.environ, {"TM_MAC_RUNNER": "runner"}), mock.patch.object(release_turn.subprocess, "Popen", return_value=run):
            return release_turn.mac_checks(Path("."))

    def test_each_exit_says_what_happened(self):
        self.assertEqual(self.status(0), (False, ""))
        self.assertEqual(self.status(1)[0], True)
        self.assertEqual(self.status(2), (False, "the runner could not run them"))
        with mock.patch.object(release_turn, "BUSY_WAIT_S", 0):
            self.assertEqual(self.status(3), (False, "the runner is busy, or its lock is stale"))

    def test_a_busy_runner_is_tried_again(self):
        runs = [mock.Mock(wait=mock.Mock(return_value=3)), mock.Mock(wait=mock.Mock(return_value=0))]
        with mock.patch.dict(os.environ, {"TM_MAC_RUNNER": "runner"}), mock.patch.object(release_turn.subprocess, "Popen", side_effect=runs), \
                mock.patch.object(release_turn.time, "sleep") as slept:
            self.assertEqual(release_turn.mac_checks(Path(".")), (False, ""))
        slept.assert_called_once()

    def test_no_runner_is_said(self):
        with mock.patch.dict(os.environ):
            os.environ.pop("TM_MAC_RUNNER", None)
            self.assertEqual(release_turn.mac_checks(Path(".")), (False, "no TM_MAC_RUNNER"))

    def test_a_runner_gone_quiet_is_ended_and_said(self):
        run = mock.Mock(pid=4242)
        run.wait.side_effect = [subprocess.TimeoutExpired("x", 1), 0]
        with mock.patch.dict(os.environ, {"TM_MAC_RUNNER": "runner"}), mock.patch.object(release_turn.subprocess, "Popen", return_value=run), \
                mock.patch.object(release_turn.os, "killpg") as ended:
            failed, why = release_turn.mac_checks(Path("."))
        ended.assert_called_once_with(4242, release_turn.signal.SIGTERM)
        self.assertFalse(failed)
        self.assertIn("minutes", why)


if __name__ == "__main__":
    unittest.main()
