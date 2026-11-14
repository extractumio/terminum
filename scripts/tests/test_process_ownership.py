# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102: cleanup queries metadata, never environments or path mentions in commands."""
import os
import contextlib
import io
from pathlib import Path
import signal
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_step
import worktree
import process_cwd


class Ownership(unittest.TestCase):
    def test_ps_never_requests_environments_on_either_platform(self):
        for platform in ("darwin", "linux"):
            with self.subTest(platform=platform), mock.patch.object(sys, "platform", platform), \
                    mock.patch.object(check_step.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "42 7 S sleep 60\n")) as run:
                self.assertEqual(list(check_step.listing()), [(42, "7", "sleep 60")])
                args = run.call_args.args[0]
                self.assertEqual(args, ["ps", "-axo", "pid=,pgid=,stat=,command="])

    def test_ownership_uses_group_and_directory_not_command_mentions(self):
        with mock.patch.object(check_step, "listing", return_value=[
                (42, "7", "sleep 60"), (43, "8", "program /tmp/private"), (44, "8", "detached")]), \
                mock.patch.object(check_step, "working_in", return_value={44: "detached"}, create=True), \
                mock.patch.object(check_step.owned_children, "active", return_value={}):
            self.assertEqual(dict(check_step.own_processes("/tmp/private", 7)), {42: "sleep 60", 44: "detached"})

    def test_a_failed_process_query_is_not_a_clean_run(self):
        with mock.patch.object(check_step.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "ps")):
            with self.assertRaises(subprocess.CalledProcessError):
                list(check_step.listing())

    def test_mac_directory_lookup_preserves_boundaries_and_excludes_itself(self):
        folder = str(Path("/tmp/private").resolve())
        output = f"p42\ncown\nn{folder}/remote\np43\ncother\nn{folder}-other\np{os.getpid()}\ncself\nn{folder}\n"
        with mock.patch.object(worktree.Path, "exists", return_value=False), \
                mock.patch.object(worktree.shutil, "which", return_value="/usr/sbin/lsof"), \
                mock.patch.object(worktree.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)) as run:
            self.assertEqual(worktree.working_in("/tmp/private"), {42: "own"})
            self.assertEqual(run.call_args.args[0], ["lsof", "-a", "-d", "cwd", "-F", "pcn"])
            self.assertTrue(run.call_args.kwargs["check"])
            self.assertLessEqual(run.call_args.kwargs["timeout"], 30)

    def test_mac_directory_lookup_failure_is_not_an_empty_list(self):
        with mock.patch.object(worktree.Path, "exists", return_value=False), \
                mock.patch.object(worktree.shutil, "which", return_value="/usr/sbin/lsof"), \
                mock.patch.object(worktree.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "lsof")):
            with self.assertRaises(process_cwd.InspectionError):
                worktree.working_in("/tmp/private")

    def test_a_timeout_retries_once_and_discards_partial_output(self):
        folder = Path("/tmp/private").resolve()
        timeout = subprocess.TimeoutExpired("lsof", 15, output=f"p42\ncworker\nn{folder}\n")
        complete = subprocess.CompletedProcess([], 0, f"p43\ncworker\nn{folder}\n")
        with mock.patch.object(process_cwd.Path, "exists", return_value=False), \
                mock.patch.object(process_cwd.shutil, "which", return_value="lsof"), \
                mock.patch.object(process_cwd.subprocess, "run", side_effect=[timeout, complete]) as run:
            self.assertEqual(worktree.working_in("/tmp/private"), {43: "worker"})
            self.assertEqual(run.call_count, 2)
            self.assertTrue(all(call.kwargs["timeout"] == 15 for call in run.call_args_list))

    def test_two_timeouts_are_an_explicit_unknown_state(self):
        with mock.patch.object(process_cwd.Path, "exists", return_value=False), \
                mock.patch.object(process_cwd.shutil, "which", return_value="lsof"), \
                mock.patch.object(process_cwd.subprocess, "run", side_effect=subprocess.TimeoutExpired("lsof", 15)) as run:
            with self.assertRaisesRegex(process_cwd.InspectionError, "timed out"):
                worktree.working_in("/tmp/private")
            self.assertEqual(run.call_count, 2)

    def test_empty_or_invalid_metadata_never_authorizes_cleanup(self):
        for output in ("", "n/tmp/private\n", "p42\nn/tmp/private\n", "p42\ncworker\nnrelative\n"):
            with self.subTest(output=output), \
                    mock.patch.object(process_cwd.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, output)) as run:
                with self.assertRaises(process_cwd.InspectionError):
                    process_cwd.mac_directories()
                run.assert_called_once()

    def test_directory_failure_preserves_only_proven_group_and_registered_processes(self):
        with mock.patch.object(check_step, "listing", return_value=[
                (42, "7", "group"), (43, "8", "unrelated /tmp/private"), (44, "8", "registered")]), \
                mock.patch.object(check_step, "working_in", side_effect=process_cwd.InspectionError("query failed")), \
                mock.patch.object(check_step.owned_children, "active", return_value={44}):
            with self.assertRaisesRegex(check_step.OwnershipError, "query failed") as error:
                check_step.own_processes("/tmp/private", 7)
            self.assertEqual(error.exception.processes, [(42, "group"), (44, "registered")])

    def test_cancellation_during_failed_inspection_keeps_original_signal_and_grace(self):
        events = []
        cancelled = check_step.GuardCancellation()
        def fail(_folder):
            cancelled.receive(signal.SIGHUP, None)
            raise process_cwd.InspectionError("query timed out")
        with mock.patch.object(check_step, "run", return_value=(mock.Mock(), 0, 7)), \
                mock.patch.object(check_step, "working_in", side_effect=fail), \
                mock.patch.object(check_step, "listing", return_value=[(42, "7", "owned")]), \
                mock.patch.object(check_step.owned_children, "active", return_value={}), \
                mock.patch.object(check_step.owned_children, "cleanup_identities", return_value={42: "start"}), \
                mock.patch.object(check_step.os, "killpg", side_effect=lambda group, number: events.append((group, number))), \
                mock.patch.object(check_step.time, "sleep", side_effect=lambda seconds: events.append(("grace", seconds))), \
                mock.patch.object(check_step, "end_pids", side_effect=lambda *_: events.append("end")), \
                mock.patch.object(check_step, "files_in", return_value=["x"]), \
                mock.patch.object(check_step.shutil, "rmtree") as remove, contextlib.redirect_stderr(io.StringIO()) as err:
            self.assertNotEqual(check_step.check(["unused"], "private", cancelled), 0)
            self.assertEqual(events, [(7, signal.SIGHUP), ("grace", check_step.SETTLE_S), "end"])
            self.assertIn("LEFT x", err.getvalue())
            self.assertIn("inspection failed", err.getvalue())
            remove.assert_not_called()

    def test_linux_directory_lookup_preserves_boundaries_and_excludes_itself(self):
        folder = Path("/tmp/private").resolve()
        entries = [Path("/proc/42"), Path("/proc/43"), Path(f"/proc/{os.getpid()}")]
        directories = {"42": str(folder / "remote"), "43": str(folder) + "-other", str(os.getpid()): str(folder)}
        with mock.patch.object(worktree.Path, "resolve", return_value=folder), \
                mock.patch.object(worktree.Path, "exists", return_value=True), \
                mock.patch.object(worktree.Path, "iterdir", return_value=entries), \
                mock.patch.object(worktree.os, "readlink", side_effect=lambda p: directories[p.parent.name]), \
                mock.patch.object(worktree.Path, "read_text", return_value="worker"):
            self.assertEqual(worktree.working_in(folder), {42: "worker"})

    def test_linux_disappearance_and_other_users_are_distinct_from_query_failure(self):
        folder = Path("/tmp/private").resolve()
        cases = [(FileNotFoundError(), os.getuid(), False), (ProcessLookupError(), os.getuid(), False),
                 (PermissionError(), os.getuid() + 1, False), (PermissionError(), os.getuid(), True),
                 (OSError("query failed"), os.getuid(), True)]
        for error, uid, fails in cases:
            with self.subTest(error=type(error), uid=uid), \
                    mock.patch.object(worktree.Path, "resolve", return_value=folder), \
                    mock.patch.object(worktree.Path, "exists", return_value=True), \
                    mock.patch.object(worktree.Path, "iterdir", return_value=[Path("/proc/42")]), \
                    mock.patch.object(worktree.os, "readlink", side_effect=error), \
                    mock.patch.object(worktree.Path, "stat", return_value=SimpleNamespace(st_uid=uid)):
                if fails:
                    with self.assertRaises(process_cwd.InspectionError):
                        worktree.working_in(folder)
                else:
                    self.assertEqual(worktree.working_in(folder), {})


if __name__ == "__main__":
    unittest.main()
