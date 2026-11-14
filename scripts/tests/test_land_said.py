# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""What a landing says (spec AC-104, AC-146): GitHub CI's last verdict on main, and what its checks skipped."""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
import test_land  # noqa: E402
from test_land import land  # noqa: E402


class CiLine(test_land.Repos, unittest.TestCase):
    real_ci = staticmethod(land.ci_status)

    def ci(self, *, stdout="", code=0, error=None):
        """land's ci_status over a stand-in for gh: its answer, or the error running it raised."""
        def run(args, **kwargs):
            self.assertEqual(args[:3], ["gh", "run", "list"])
            if error:
                raise error
            return subprocess.CompletedProcess(args, code, stdout, "HTTP 401: Bad credentials\n" if code else "")
        return self.real_ci(self.primary, run=run)

    def test_the_last_ci_run_on_main_is_said_with_its_commit(self):  # AC-104
        head = self.on_origin().strip()
        said = self.ci(stdout=f'[{{"status":"completed","conclusion":"failure","headSha":"{head}","url":"https://ci.example/1"}}]')
        self.assertEqual(said, f"CI on main: failure on {head[:9]}: https://ci.example/1")

    def test_a_run_of_an_older_commit_or_one_still_running_says_so(self):  # AC-104
        head = self.on_origin().strip()
        said = self.ci(stdout='[{"status":"in_progress","conclusion":"","headSha":"abcdef0123456789","url":"u2"},'
                              '{"status":"completed","conclusion":"failure","headSha":"0123456789abcdef","url":"u"}]')
        self.assertEqual(said, f"CI on main: failure on 012345678, not origin/main's {head[:9]}: u; newer: in_progress on abcdef012")

    def test_the_verdict_is_the_last_run_that_ended_a_newer_one_said_after_it(self):  # AC-104
        head = self.on_origin().strip()
        said = self.ci(stdout=f'[{{"status":"queued","conclusion":"","headSha":"{head}","url":"u3"}},'
                              '{"status":"completed","conclusion":"cancelled","headSha":"1111111111","url":"u2"},'
                              '{"status":"completed","conclusion":"success","headSha":"2222222222","url":"u1"}]')
        self.assertEqual(said, f"CI on main: success on 222222222, not origin/main's {head[:9]}: u1; newer: queued on {head[:9]}")
        only_cancelled = self.ci(stdout='[{"status":"completed","conclusion":"cancelled","headSha":"1111111111","url":"u2"}]')
        self.assertEqual(only_cancelled, "CI on main: no verdict in the last 1 runs; newest: cancelled on 111111111: u2")

    def test_without_gh_or_an_answer_the_status_is_unknown_and_never_an_error(self):  # AC-104
        self.assertEqual(self.ci(error=FileNotFoundError(2, "No such file or directory", "gh")),
                         "CI on main: unknown ([Errno 2] No such file or directory: 'gh')")
        self.assertEqual(self.ci(code=1), "CI on main: unknown (HTTP 401: Bad credentials)")
        self.assertIn("unknown (Command", self.ci(error=subprocess.TimeoutExpired("gh", 10)))
        self.assertEqual(self.ci(stdout="[]"), "CI on main: no run")
        self.assertIn("unknown (gh's answer: JSONDecodeError", self.ci(stdout="<html>rate limited</html>"))
        self.assertIn("CI on main: unknown (gh's answer: KeyError(", self.ci(stdout='[{"status":"completed"}]'))


class Said(unittest.TestCase):
    def test_timed_checks_a_busy_machine_skipped_are_said_after_landed_though_the_install_took_the_tree(self):  # AC-146
        with tempfile.TemporaryDirectory() as tmp:
            primary = Path(tmp)
            tree = primary / ".worktrees" / "x"
            (tree / ".checks").mkdir(parents=True)
            (tree / ".checks" / "skipped").write_text("targeted: SKIPPED timed checks, the machine busy for 15 minutes: page\n")
            out = []
            with mock.patch.object(land, "primary_of", return_value=primary), mock.patch.object(land, "git"), \
                    mock.patch.object(land, "ci_status", return_value="CI on main: success on 0123456789: u"), \
                    mock.patch.object(land, "land", lambda tree, after: after(["x.rs"]) or "abcdef0123456789"), \
                    mock.patch.object(land, "after_push", lambda _, files: shutil.rmtree(tree)), mock.patch.object(land.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)), \
                    mock.patch.object(land.os, "chdir"), mock.patch.object(land, "install_pending"), mock.patch("builtins.print", lambda *a, **_: out.append(" ".join(map(str, a)))):
                self.assertEqual(land.main(["x"]), 0)
        self.assertEqual(out[-2:], ["landed abcdef012 on origin/main",
                                    "targeted: SKIPPED timed checks, the machine busy for 15 minutes: page"])


if __name__ == "__main__":
    unittest.main()
