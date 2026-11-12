# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102: cleanup queries metadata, never environments or path mentions in commands."""
import os
from pathlib import Path
import subprocess
import sys
from types import SimpleNamespace
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_step
import worktree


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
                mock.patch.object(check_step, "working_in", return_value={44: "detached"}, create=True):
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
            with self.assertRaises(subprocess.CalledProcessError):
                worktree.working_in("/tmp/private")

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
                    with self.assertRaises(type(error)):
                        worktree.working_in(folder)
                else:
                    self.assertEqual(worktree.working_in(folder), {})


if __name__ == "__main__":
    unittest.main()
