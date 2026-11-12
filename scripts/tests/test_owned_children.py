# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102: registered children retain ownership outside a private cwd; reused PIDs are never signalled."""
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_step
import owned_children
from testrun import short_tmp


class RegisteredChildren(unittest.TestCase):
    def test_identity_queries_start_and_state_only_and_ignores_zombies(self):
        for state, expected in [("S", "Thu Nov 12 21:00:00 2026"), ("Z", None)]:
            with self.subTest(state=state), mock.patch.object(owned_children.subprocess, "run",
                    return_value=subprocess.CompletedProcess([], 0, f"42 Thu Nov 12 21:00:00 2026 {state}\n")) as run:
                self.assertEqual(owned_children.identity(42), expected)
                self.assertEqual(run.call_args.args[0], ["ps", "-p", "42", "-o", "pid=,lstart=,stat="])

    def test_registry_is_private_and_filters_changed_identities(self):
        with tempfile.TemporaryDirectory(prefix="tm-child-", dir=short_tmp()) as folder:
            registry = owned_children.path(folder)
            with mock.patch.object(owned_children, "identity", return_value="original"):
                owned_children.register(registry, 42)
                with mock.patch.object(owned_children, "identities", return_value={42: "original"}):
                    self.assertEqual(owned_children.active(registry), {42})
            self.assertEqual(os.stat(registry).st_mode & 0o077, 0)
            with mock.patch.object(owned_children, "identities", return_value={42: "reused"}):
                self.assertEqual(owned_children.active(registry), set())
            self.assertEqual(check_step.files_in(folder), [], "the guard's registry is not a leftover")

    def test_discovery_batches_historical_records_in_one_query(self):
        with mock.patch.object(owned_children, "records", return_value={42: "original", 43: "ended", 44: "old"}), \
                mock.patch.object(owned_children.subprocess, "run", return_value=subprocess.CompletedProcess([], 0,
                    "42 original S\n43 ended Z\n44 reused S\n")) as run:
            self.assertEqual(owned_children.active("registry"), {42})
            run.assert_called_once()
            self.assertEqual(run.call_args.args[0], ["ps", "-p", "42,43,44", "-o", "pid=,lstart=,stat="])

    def test_stale_pid_is_not_signalled(self):
        with mock.patch.object(owned_children, "identity", return_value="reused"), mock.patch.object(check_step, "send") as send:
            check_step.end_pids([42], {42: "original"})
            send.assert_not_called()

    def test_identity_is_rechecked_before_sigkill(self):
        with mock.patch.object(owned_children, "identity", side_effect=["original", "reused"]), \
                mock.patch.object(check_step, "send", return_value=True) as send, \
                mock.patch.object(check_step.time, "monotonic", side_effect=[0, 3, 3, 6]):
            check_step.end_pids([42], {42: "original"})
            send.assert_called_once_with(42, signal.SIGTERM)

    def test_failed_identity_query_is_not_an_ended_process(self):
        with mock.patch.object(owned_children.subprocess, "run", return_value=subprocess.CompletedProcess([], 2, "")):
            with self.assertRaises(subprocess.CalledProcessError):
                owned_children.identity(42)


if __name__ == "__main__":
    unittest.main()
