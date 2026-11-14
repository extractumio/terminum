# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Heavy-build slots shared by a repository's trees (spec AC-104), on a real repository in a temporary folder."""
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import heavy_slots  # noqa: E402
from worktree import Turn  # noqa: E402


class Slots(unittest.TestCase):
    def setUp(self):
        home = tempfile.TemporaryDirectory()
        self.addCleanup(home.cleanup)
        self.repo = Path(home.name) / "tree"
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)

    def test_how_many_by_cores_or_the_variable(self):  # AC-104
        with mock.patch.dict("os.environ", {}, clear=True), mock.patch("os.cpu_count", return_value=16):
            self.assertEqual(heavy_slots.count(), 2)
        with mock.patch.dict("os.environ", {}, clear=True), mock.patch("os.cpu_count", return_value=4):
            self.assertEqual(heavy_slots.count(), 1, "at least one")
        for value, slots in (("3", 3), ("0", 1), ("many", 1)):
            with mock.patch.dict("os.environ", {"TM_HEAVY_SLOTS": value}), mock.patch("os.cpu_count", return_value=8):
                self.assertEqual(heavy_slots.count(), slots, value)

    def test_the_landing_has_a_slot_of_its_own_beside_the_trees_at_work(self):  # AC-104
        with mock.patch.dict("os.environ", {"TM_HEAVY_SLOTS": "1"}), Turn(self.repo, "terminum-heavy.0.slot") as held:
            self.assertTrue(held.take())
            with heavy_slots.slot(self.repo, "cargo", landing=True, sleep=self.fail):
                with Turn(self.repo, "terminum-heavy.land.slot") as other:
                    self.assertFalse(other.take(), "its own, held while it builds")

    def test_a_cargo_step_waits_for_a_free_slot_says_who_holds_them_and_names_itself(self):  # AC-104
        said, inside = [], threading.Event()
        with mock.patch.dict("os.environ", {"TM_HEAVY_SLOTS": "1"}):
            other = Turn(self.repo, "terminum-heavy.0.slot")
            other.__enter__()
            self.assertTrue(other.take())
            other.hold_as("two cargo")

            def build():
                with heavy_slots.slot(self.repo, "cargo", say=said.append, sleep=lambda _: threading.Event().wait(0.05)):
                    inside.set()
                    self.holder = other.holder()
            runner = threading.Thread(target=build)
            runner.start()
            self.assertFalse(inside.wait(0.5), "waits while the only slot is held")
            other.__exit__()
            self.assertTrue(inside.wait(5), "goes on once it is free")
            runner.join(5)
        self.assertEqual(said, ["slot: cargo waits for one of 1 heavy-build slots: two cargo"], "said once")
        self.assertEqual(self.holder, "tree cargo")

    def test_a_slot_is_free_once_its_step_ends(self):  # AC-104
        with mock.patch.dict("os.environ", {"TM_HEAVY_SLOTS": "1"}):
            with heavy_slots.slot(self.repo, "debug-build"):
                pass
            with Turn(self.repo, "terminum-heavy.0.slot") as turn:
                self.assertTrue(turn.take())


if __name__ == "__main__":
    unittest.main()
