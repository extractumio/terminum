# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-196: the real rotation poll survives a renamed path without hiding other I/O errors."""
from pathlib import Path
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from e2e_logs import rotation_state
from e2e_ws import wait_for
from testrun import short_tmp


class RotationPoll(unittest.TestCase):
    def test_missing_active_path_is_polled_again_until_the_new_files_exist(self):
        with tempfile.TemporaryDirectory(prefix="tm-log-poll-", dir=short_tmp()) as folder:
            daemon, agent, expired = (Path(folder) / name for name in ("daemon.log", "agent.log", "expired.log"))
            daemon.write_bytes(b"older daemon\n")
            agent.write_bytes(b"older agent\n")
            for moving in (daemon, agent):
                moving.rename(Path(str(moving) + ".1"))
                reads = []

                def poll():
                    state = rotation_state(daemon, agent, expired)
                    reads.append(state)
                    if state is None:
                        moving.write_bytes(b"new\n")
                    return state

                result = wait_for(poll, lambda state: state is not None, seconds=1, every=0)
                self.assertIsNone(reads[0])
                self.assertEqual(len(reads), 2)
                self.assertEqual(result[0 if moving == daemon else 1], 4)
                self.assertFalse(result[2])

    def test_a_metadata_permission_failure_is_reported(self):
        with mock.patch.object(Path, "stat", side_effect=PermissionError("fixture permission failure")):
            with self.assertRaisesRegex(PermissionError, "fixture permission failure"):
                rotation_state(Path("daemon.log"), Path("agent.log"), Path("expired.log"))
