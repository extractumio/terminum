# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""scripts/gate_retry.py (spec AC-201): which red checks the full run retries, how, and what it says; on real
repositories in a temporary folder, the logs written by hand."""
import os
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gate_record  # noqa: E402
import gate_retry  # noqa: E402
from test_gate_record import Repo  # noqa: E402


def log(folder, *lines):
    path = Path(folder) / "step.log"
    path.write_text("".join(f"{line}\n" for line in lines))
    return path


class Reading(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name)

    def test_red_checks_are_named_where_the_step_can_name_them(self):  # AC-201
        e2e = log(self.dir, "PASS AC-1 x", "FAIL AC-39 a hook", "FAIL scripts/e2e_hooks.py", "FAIL scripts/e2e_core.py")
        self.assertEqual(gate_retry.red_checks("e2e", e2e), ["scripts/e2e_hooks.py", "scripts/e2e_core.py"])
        page = log(self.dir, "FAIL [webkit] x", "red checks: webBoardPanel webTerm", "FAIL (2 of 90)")
        self.assertEqual(gate_retry.red_checks("page", page), ["webBoardPanel", "webTerm"])
        self.assertEqual(gate_retry.red_checks("live", page), ["live"], "a suite")
        self.assertEqual(gate_retry.red_checks("page", log(self.dir, "thrown")), ["page"], "none named: the suite")

    def test_the_retry_runs_only_the_red_checks(self):  # AC-201
        self.assertEqual(gate_retry.again("page", "node web/test/e2e_all.mjs", ["webTerm"]), "node web/test/e2e_all.mjs webTerm")
        self.assertIn("'scripts/e2e_hooks.py'", gate_retry.again("e2e", "all of them", ["scripts/e2e_hooks.py"]))
        self.assertNotIn("e2e_core", gate_retry.again("e2e", "all of them", ["scripts/e2e_hooks.py"]))
        self.assertEqual(gate_retry.again("live", "node web/test/e2e_live.mjs", ["live"]), "node web/test/e2e_live.mjs")

    def test_a_crash_is_named_by_file_names_alone(self):  # AC-201
        reports = self.dir / "reports"
        reports.mkdir()
        began = time.time() - 5
        (reports / "tmux-2026-11-18-031244.ips").write_text("secret contents")
        old = reports / "node-old.ips"
        old.write_text("x")
        os.utime(old, (began - 100, began - 100))
        (reports / "Finder-x.ips").write_text("x")
        quiet = log(self.dir, "FAIL x")
        self.assertEqual(gate_retry.crash(began, quiet, reports), "tmux-2026-11-18-031244.ips")
        self.assertIsNone(gate_retry.crash(time.time() + 10, quiet, reports))
        gone = log(self.dir, "server exited unexpectedly")
        self.assertEqual(gate_retry.crash(began, gone, self.dir / "none"), "tmux's server gone")


class Deciding(Repo, unittest.TestCase):
    def setUp(self):
        self.make_repo()
        self.commit("scripts/e2e_hooks.py", "# AC-39\nif __name__ == \"__main__\":\n    pass\n")
        self.commit("web/test/e2e_all.mjs", 'import { webBoardPanel } from "./e2e_boardpanel.mjs";\n')
        self.passed = self.commit("web/test/e2e_boardpanel.mjs", "// AC-111\n")
        self.pass_whole(self.passed)
        self.retry = gate_retry.Retry(self.repo, "c")
        self.reports = mock.patch.object(gate_retry, "CRASH_REPORTS", self.root / "none")
        self.reports.start()
        self.addCleanup(self.reports.stop)

    def decide(self, step, *lines):
        with mock.patch("builtins.print"):
            return self.retry.decide(step, "node web/test/e2e_all.mjs", log(self.root, *lines), time.time())

    def test_a_flaky_check_of_untouched_code_runs_once_more(self):  # AC-201
        self.commit("docs/x.md")
        command, checks, entry = self.decide("page", "FAIL [chromium] AC-111 Find", "red checks: webBoardPanel")
        self.assertEqual((command, checks), ("node web/test/e2e_all.mjs webBoardPanel", ["webBoardPanel"]))
        self.assertEqual(entry["cause"], "failed twice")

    def test_no_retry_for_a_check_of_code_changed_since_the_last_full_pass(self):  # AC-201
        self.commit("web/test/e2e_boardpanel.mjs", "// AC-111 changed\n")
        command, _, entry = self.decide("page", "red checks: webBoardPanel")
        self.assertIsNone(command)
        self.assertIn(f"webBoardPanel: its code changed since {self.passed[:9]}'s full pass", entry["cause"])

    def test_no_retry_when_a_red_line_names_a_case_the_change_touched(self):  # AC-201
        self.commit("terminumd/src/hooks.rs", "// AC-39\nfn x() {}\n")
        command, _, entry = self.decide("live", "FAIL [chromium] AC-39 the hook's state shows")
        self.assertIsNone(command)
        self.assertIn("AC-39 changed since", entry["cause"])

    def test_no_retry_for_a_check_retried_in_3_of_the_last_7_runs(self):  # AC-201
        for _ in range(3):
            gate_record.append(self.repo, {"kind": "gate", "steps": {"e2e": {"retried": ["e2e scripts/e2e_hooks.py"]}}})
        command, _, entry = self.decide("e2e", "FAIL scripts/e2e_hooks.py")
        self.assertIsNone(command)
        self.assertIn("retried in 3 of the last 7 runs", entry["cause"])

    def test_steps_of_no_real_program_never_retry(self):  # AC-201
        for step in ("test", "clippy", "install", "security", "build"):
            self.assertEqual(self.decide(step, "FAIL x")[0], None, step)

    def test_a_pass_on_retry_is_said_last_with_its_task(self):  # AC-201
        first = log(self.root, "the first run's line")
        self.retry.passed_on_retry("page", ["webBoardPanel"], first)
        with mock.patch("builtins.print") as said:
            self.retry.say_last()
        words = "".join(str(call.args[0]) for call in said.call_args_list)
        self.assertIn('"Bug: flaky in the full run: page webBoardPanel" (P2)', words)
        self.assertIn("the first run's line", words)
        self.assertIn("PASSED ONLY ON RETRY, the first run red: page webBoardPanel", words)

    def test_the_whole_pass_keeps_what_its_steps_kept(self):  # AC-201
        gate_record.keep(self.repo, "c", {"retried": ["page webBoardPanel"]}, "page")
        gate_record.keep(self.repo, "c", {"skip": "Docker does not answer"}, "docker")
        gate_record.keep(self.repo, "c", {}, "live")
        self.assertEqual(gate_retry.whole_notes(self.repo, "c", ["live", "page", "docker"]),
                         {"retried": ["page webBoardPanel"], "skip": "Docker does not answer"})


if __name__ == "__main__":
    unittest.main()
