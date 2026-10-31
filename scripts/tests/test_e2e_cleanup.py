# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The orphan-fixture cleanup ends only a process group explicitly created by this test."""
import importlib.util
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
SPEC = importlib.util.spec_from_file_location("hostattach", SCRIPTS / "e2e_hostattach.py")
hostattach = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(hostattach)


class FixtureCleanup(unittest.TestCase):
    def test_live_fixture_cleanup_ends_its_background_pipeline(self):
        with tempfile.TemporaryDirectory(prefix="tm-fixture-") as folder:
            pidfile = Path(folder) / "child.pid"
            process = subprocess.Popen(["/bin/sh", "-c", 'sleep 60 & echo $! > "$1"; wait', "fixture", str(pidfile)],
                                       start_new_session=True)
            hostattach.fixtures.append(process)
            try:
                until = time.monotonic() + 3
                while not pidfile.exists() and time.monotonic() < until:
                    time.sleep(0.01)
                pid = int(pidfile.read_text())
                hostattach.cleanup_fixtures()
                self.assertIsNotNone(process.poll())
                state = subprocess.run(["/bin/ps", "-o", "stat=", "-p", str(pid)], capture_output=True,
                                       text=True, timeout=3).stdout.strip()
                self.assertTrue(not state or state.startswith("Z"), state)
                self.assertEqual(hostattach.fixtures, [])
            finally:
                hostattach.cleanup_fixtures()
