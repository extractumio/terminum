# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Live-check pools shared by repository trees (spec AC-104), using real kernel locks."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import live_slots  # noqa: E402
from testrun import short_tmp  # noqa: E402
from worktree import Turn  # noqa: E402


class Slots(unittest.TestCase):
    def setUp(self):
        home = tempfile.TemporaryDirectory(prefix="tms-", dir=short_tmp())
        self.addCleanup(home.cleanup)
        self.repo = Path(home.name) / "tree"
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)

    def held(self, name):
        with Turn(self.repo, f"terminum-{name}.slot") as other:
            return not other.take()

    def test_default_and_overrides_are_independent_of_the_heavy_pool(self):
        with mock.patch.dict(os.environ, {"TM_HEAVY_SLOTS": "7"}, clear=True):
            self.assertEqual(live_slots.count(), 2)
            for value, expected in (("3", 3), ("0", 1), ("-2", 1), ("many", 2), ("", 2)):
                with self.subTest(value=value), mock.patch.dict(os.environ, {"TM_LIVE_SLOTS": value}):
                    self.assertEqual(live_slots.count(), expected)

    def test_a_waiter_names_the_holders_once_and_starts_only_after_release(self):
        said, sleeps = [], []
        with mock.patch.dict(os.environ, {"TM_LIVE_SLOTS": "1"}), Turn(self.repo, "terminum-live.0.slot") as held:
            self.assertTrue(held.take())
            held.hold_as("other panel")

            def wait(seconds):
                self.assertTrue(self.held("live.0"))
                sleeps.append(seconds)
                if len(sleeps) == 2:
                    held.file.close()

            with live_slots.slot(self.repo, "critical", say=said.append, sleep=wait):
                self.assertTrue(self.held("live.0"))
                self.assertEqual(held.holder(), "tree critical")
        self.assertEqual(len(sleeps), 2)
        self.assertEqual(said, ["slot: critical waits for a live-check slot (1 slot): other panel"])
        self.assertFalse(self.held("live.0"))

    def test_another_free_slot_is_used_without_waiting(self):
        with mock.patch.dict(os.environ, {"TM_LIVE_SLOTS": "2"}), Turn(self.repo, "terminum-live.0.slot") as held:
            self.assertTrue(held.take())
            with live_slots.slot(self.repo, "page", sleep=self.fail):
                self.assertTrue(self.held("live.1"))
            self.assertFalse(self.held("live.1"))

    def test_landing_and_heavy_slots_do_not_consume_worker_live_slots(self):
        with mock.patch.dict(os.environ, {"TM_LIVE_SLOTS": "1"}), \
                Turn(self.repo, "terminum-heavy.0.slot") as heavy:
            self.assertTrue(heavy.take())
            with live_slots.slot(self.repo, "smoke", landing=True, sleep=self.fail):
                self.assertTrue(self.held("live.land"))
                with live_slots.slot(self.repo, "critical", sleep=self.fail):
                    self.assertTrue(self.held("live.0"))

    def test_a_spawned_program_does_not_inherit_the_slot(self):
        child = None
        try:
            with mock.patch.dict(os.environ, {"TM_LIVE_SLOTS": "1"}):
                with live_slots.slot(self.repo, "critical"):
                    child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"], close_fds=False)
                self.assertFalse(self.held("live.0"), "free even while the child is alive")
                self.assertIsNone(child.poll())
        finally:
            if child is not None:
                child.terminate()
                child.wait(timeout=10)

    def test_the_kernel_frees_a_slot_when_its_holder_dies(self):
        ready = self.repo / "ready"
        code = ("import sys,time; from pathlib import Path; sys.path.insert(0,sys.argv[1]); import live_slots\n"
                "with live_slots.slot(Path(sys.argv[2]),'critical'):\n"
                " Path(sys.argv[3]).touch(); time.sleep(60)\n")
        with mock.patch.dict(os.environ, {"TM_LIVE_SLOTS": "1"}):
            child = subprocess.Popen([sys.executable, "-c", code, str(Path(live_slots.__file__).parent), str(self.repo), str(ready)])
            try:
                deadline = time.monotonic() + 10
                while not ready.exists() and child.poll() is None and time.monotonic() < deadline:
                    time.sleep(0.02)
                self.assertTrue(ready.exists(), "the holder reached its slot")
                self.assertTrue(self.held("live.0"))
            finally:
                child.kill()
                child.wait(timeout=10)
            self.assertFalse(self.held("live.0"))


if __name__ == "__main__":
    unittest.main()
