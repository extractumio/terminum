# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""scripts/check_all.py, the full run (spec AC-201, AC-146, AC-104): which steps run on a content, what they see of
the environment, the retry after every first run, the record. On a real repository in a temporary folder, each step's
command stood in for, so no check of the real tree runs."""
import json
import os
from pathlib import Path
import sys
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_all  # noqa: E402
import gate_record  # noqa: E402
import gate_retry  # noqa: E402
import load_gate  # noqa: E402
from test_gate_record import Repo  # noqa: E402

STEPS = [name for name, _ in check_all.steps(Path("/nowhere"), "x")]


class Gate(Repo, unittest.TestCase):
    def setUp(self):
        self.make_repo()
        self.head = self.commit("a.txt")
        self.red, self.again, self.ran, self.envs = set(), set(), [], []
        self.lines = {}
        self.busy = 0
        patches = [mock.patch.object(check_all, "REPO", self.repo), mock.patch.object(check_all, "attempt", self.attempt),
                   mock.patch.object(check_all.load_gate, "main", lambda argv: self.busy),
                   mock.patch.object(gate_retry, "CRASH_REPORTS", self.root / "none"), mock.patch("builtins.print", self.say)]
        for patch in patches:
            patch.start()
            self.addCleanup(patch.stop)
        self.said = []

    def say(self, *words, **_):
        self.said.append(" ".join(str(w) for w in words))

    def attempt(self, root, log, command, env):
        name = log.name.split(".")[1]
        self.ran.append(name)
        self.envs.append(env)
        log.write_text("".join(f"{line}\n" for line in self.lines.get(name, [])))
        if name in self.red or name in self.again and self.ran.count(name) == 1:
            return 1, 3
        return 0, 3

    def check(self, *argv):
        self.ran = []
        return check_all.main(list(argv))

    def test_a_second_run_runs_the_red_step_only(self):  # AC-201
        self.red = {"clippy"}
        self.assertEqual(self.check(), 1)
        self.assertEqual(self.ran, ["build", *STEPS], "the build first, then every step")
        self.red = set()
        self.assertEqual(self.check(), 0)
        self.assertEqual(self.ran, ["build", "clippy"])
        self.assertIn("skip test: passed on", "\n".join(self.said))
        self.assertEqual(self.check(), 0)
        self.assertEqual(self.ran, [], "passed already: nothing runs")
        self.assertIn("check: passed already on", self.said[-1])
        self.assertEqual(self.check("--force"), 0)
        self.assertEqual(self.ran, ["build", *STEPS])

    def test_a_docs_only_change_since_a_full_pass_runs_the_scan_alone(self):  # AC-201
        self.assertEqual(self.check(), 0)
        self.commit("docs/releases/v1.0.0.md")
        self.assertEqual(self.check(), 0)
        self.assertEqual(self.ran, ["security"], "no build either")
        self.commit("README.md")   # the package carries it
        self.assertEqual(self.check(), 0)
        self.assertEqual(self.ran, ["build", *STEPS])

    def test_a_step_sees_only_the_allow_list(self):  # AC-201
        caller = {"PATH": os.environ["PATH"], "HOME": "/Users/alex", "CLAUDECODE": "1", "TM_APP_DIR": "/x", "TM_PORT": "48080",
                  "SSH_AUTH_SOCK": "/agent", "TM_TEST_NO_CLAUDE": "1", "TM_GATE_PASSED": "1", "DOCKER_CONTEXT": "colima"}
        with mock.patch.dict(os.environ, caller, clear=True):
            self.check()
        self.assertEqual(self.envs[0], {"PATH": os.environ["PATH"], "HOME": "/Users/alex", "DOCKER_CONTEXT": "colima", "TM_TIMING": "report"})
        self.assertTrue(all(env == self.envs[0] for env in self.envs))

    def test_a_busy_machine_ends_a_run_by_hand_before_any_step(self):  # AC-146
        self.busy = load_gate.BUSY
        self.assertEqual(self.check(), load_gate.BUSY)
        self.assertEqual(self.ran, [])
        self.assertEqual(self.check("--by", "release"), 0, "a release waited at its own gate")

    def test_passed_and_dockers_skip(self):  # AC-146, AC-201
        self.assertEqual(self.check("--passed"), 1)
        self.lines["docker"] = []
        self.check()
        self.assertEqual(self.check("--passed"), 0)
        ident = check_all.tree_id(self.repo)
        gate_record.keep(self.repo, ident, {"skip": "Docker does not answer"})
        with mock.patch.object(check_all, "docker_can_run", return_value=False):
            self.assertEqual(self.check("--passed"), 0, "a pass without Docker counts while Docker cannot run")
        with mock.patch.object(check_all, "docker_can_run", return_value=True):
            self.assertEqual(self.check("--passed"), 1, "Docker answers now: its step has to run")

    def test_a_rehearsal_runs_every_step_and_a_release_reuses_only_young_passes(self):  # AC-201, AC-202
        self.check()
        self.assertEqual(self.check("--by", "rehearse"), 0)
        self.assertEqual(self.ran, ["build", *STEPS])
        ident = check_all.tree_id(self.repo)
        for step in [None, *STEPS]:
            for mark in gate_record.marks(self.repo, ident, step):
                mark.write_text(json.dumps({"at": "2026-01-01 00:00"}))
        self.assertEqual(self.check(), 0)
        self.assertEqual(self.ran, [], "by hand an old pass counts")
        self.assertEqual(self.check("--by", "release"), 0)
        self.assertEqual(self.ran, ["build", *STEPS], "never for a release")

    def test_a_flaky_page_check_passes_on_its_retry_after_every_first_run(self):  # AC-201
        self.again = {"page"}
        self.red = {"clippy"}
        self.lines["page"] = ["FAIL [webkit] a check", "red checks: webBoardPanel"]
        self.assertEqual(self.check(), 1, "clippy stays red")
        self.assertEqual(self.ran, ["build", *STEPS, "page"], "the retry after every first run, beside a red step")
        self.assertIn("ok   page (6 s) on its retry", self.said)
        self.assertIn("PASSED ONLY ON RETRY, the first run red: page webBoardPanel", "\n".join(self.said))
        ident = check_all.tree_id(self.repo)
        self.assertEqual(gate_record.passed(self.repo, ident, "page")["retried"], ["page webBoardPanel"])
        self.red = set()
        self.check()
        self.assertEqual(gate_record.passed(self.repo, ident)["retried"], ["page webBoardPanel"], "the release names it")

    def test_a_timing_line_is_kept_with_the_step(self):  # AC-201
        self.lines["e2e"] = ["TIMING over budget (judged on a quiet machine): AC-39 a hook (median): 62 ms, budget 50 ms"]
        self.check()
        ident = check_all.tree_id(self.repo)
        self.assertEqual(len(gate_record.passed(self.repo, ident, "e2e")["timing"]), 1)

    def test_each_run_leaves_a_line_in_the_record(self):  # AC-201
        self.red = {"live"}
        self.check("--by", "release")
        entry = gate_record.read(self.repo, "gate")[-1]
        self.assertEqual((entry["by"], entry["result"], entry["commit"]), ("release", "fail", self.head))
        self.assertEqual(entry["steps"]["live"]["result"], "fail")
        self.assertEqual(entry["steps"]["live"]["cause"], "failed twice", "live retried, red again")
        self.assertEqual(self.ran.count("live"), 2)


class Budget(unittest.TestCase):
    def test_in_budget_under_the_full_runs_load(self):  # AC-201
        from e2e_ws import in_budget
        with mock.patch.dict(os.environ, {"TM_TIMING": "report"}), mock.patch("builtins.print") as said:
            self.assertTrue(in_budget("AC-39 a hook", 0.062, 0.05))
            self.assertFalse(in_budget("AC-39 a hook", 0.25, 0.05), "five budgets: a hang")
            self.assertFalse(in_budget("AC-39 a hook", None, 0.05))
        self.assertIn("TIMING over budget (judged on a quiet machine): AC-39 a hook: 62 ms, budget 50 ms", said.call_args_list[0].args[0])
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertFalse(in_budget("AC-39 a hook", 0.062, 0.05), "judged anywhere else")
            self.assertTrue(in_budget("AC-39 a hook", 0.04, 0.05))


if __name__ == "__main__":
    unittest.main()
