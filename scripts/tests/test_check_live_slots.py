# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Live admission around real-program steps and retries (spec AC-104, AC-146, AC-197)."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_changed  # noqa: E402
from testrun import short_tmp  # noqa: E402
from worktree import Turn  # noqa: E402


class Run(unittest.TestCase):
    def setUp(self):
        home = tempfile.TemporaryDirectory(prefix="tms-", dir=short_tmp())
        self.addCleanup(home.cleanup)
        self.root = Path(home.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        self.env = mock.patch.dict("os.environ", {"TM_LIVE_SLOTS": "1"})
        self.env.start()
        self.addCleanup(self.env.stop)

    def held(self, kind="0"):
        with Turn(self.root, f"terminum-live.{kind}.slot") as other:
            return not other.take()

    def test_real_steps_hold_slots_and_other_steps_do_not(self):
        actual = {}

        def guard(_, command, **kw):
            actual[command[-1]] = self.held()
            return 0

        live = ["panel", "page", "critical", "smoke", "e2e_core", "e2e_attach", "e2e_hooks"]
        light = ["security", "docs", "scripts", "ui", "ui-unit", "page-unit", "cargo", "debug-build"]
        with mock.patch.object(check_changed, "run_guard", guard), mock.patch("sys.stdout"):
            self.assertEqual(check_changed.run([(name, name) for name in live + light], root=self.root, gate=False), [])
        self.assertEqual(actual, {**dict.fromkeys(live, True), **dict.fromkeys(light, False)})
        self.assertFalse(self.held())

    def test_the_quiet_gate_is_checked_inside_the_acquired_slot_before_the_command(self):
        events = []

        def gate(_):
            events.append("gate")
            self.assertTrue(self.held())

        def guard(*_, **kw):
            events.append("command")
            self.assertTrue(self.held())
            return 0

        with mock.patch.object(check_changed, "for_task", gate), mock.patch.object(check_changed, "run_guard", guard), \
                mock.patch("sys.stdout"):
            self.assertEqual(check_changed.run([("critical", "node live")], root=self.root), [])
        self.assertEqual(events, ["gate", "command"])
        self.assertFalse(self.held())

    def test_a_busy_gate_releases_its_slot_and_later_skips_take_none(self):
        real_slot = check_changed.live_slots.slot
        with mock.patch.object(check_changed.live_slots, "slot", wraps=real_slot) as slots, \
                mock.patch.object(check_changed, "for_task", return_value="load 31 over 16"), \
                mock.patch.object(check_changed, "run_guard") as guard, mock.patch("sys.stdout"):
            self.assertEqual(check_changed.run([("panel", "a"), ("critical", "b")], root=self.root), [])
        self.assertEqual(slots.call_count, 1)
        guard.assert_not_called()
        self.assertFalse(self.held())
        self.assertIn("panel critical", check_changed.note(self.root, "skipped"))

    def test_the_same_slot_stays_held_through_both_attempts(self):
        answers = iter([1, 0])
        real_slot = check_changed.live_slots.slot

        def guard(*_, stdout, **kw):
            self.assertTrue(self.held())
            stdout.write("a real-program check\n")
            return next(answers)

        with mock.patch.object(check_changed.live_slots, "slot", wraps=real_slot) as slots, \
                mock.patch.object(check_changed, "run_guard", guard), mock.patch("sys.stdout"):
            self.assertEqual(check_changed.run([("critical", "node live")], root=self.root, gate=False), [])
        self.assertEqual(slots.call_count, 1, "no release and re-acquire between attempts")
        self.assertFalse(self.held())
        self.assertIn("critical", check_changed.note(self.root, "retried"))

    def test_failures_errors_and_cancellation_release_the_slot(self):
        for error in (RuntimeError("guard failed"), SystemExit(143)):
            with self.subTest(error=error), mock.patch.object(check_changed, "run_guard", side_effect=error), \
                    mock.patch("sys.stdout"), self.assertRaises(type(error)):
                check_changed.run([("critical", "node live")], root=self.root, gate=False)
            self.assertFalse(self.held())
        with mock.patch.object(check_changed, "run_guard", return_value=1), mock.patch("sys.stdout"):
            self.assertEqual(check_changed.run([("critical", "false")], root=self.root, gate=False), ["critical"])
        self.assertFalse(self.held())

    def test_every_landing_live_step_uses_the_reserved_slot_with_workers_busy(self):
        commands = []

        def guard(_, command, **kw):
            self.assertTrue(self.held("land"))
            self.assertTrue(self.held("0"))
            commands.append(command[-1])
            return 0

        with Turn(self.root, "terminum-live.0.slot") as worker:
            self.assertTrue(worker.take())
            with mock.patch.object(check_changed, "run_guard", guard), mock.patch.object(check_changed, "for_task") as gate, \
                    mock.patch("sys.stdout"):
                self.assertEqual(check_changed.run([(name, name) for name in ("e2e_core", "e2e_attach", "smoke")],
                                                   root=self.root, gate=False, landing=True), [])
            gate.assert_not_called()
        self.assertEqual(commands, ["e2e_core", "e2e_attach", "smoke"])
        self.assertFalse(self.held("land"))


if __name__ == "__main__":
    unittest.main()
