# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-196: actual check-output sink, byte bounds, retained tail and failure cleanup."""
from pathlib import Path
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_logs
from testrun import short_tmp


class Logs(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix="tm-log-test-", dir=short_tmp())
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name)
        self.path = self.root / ".checks/changed.test.log"

    def test_noisy_check_rotates_and_every_byte_is_retained(self):
        data = b"one bounded line\n" * 700_000 + b"last record\n"
        with check_logs.bounded_log(self.root, self.path) as out:
            out.write(data)
        parts = [Path(str(self.path) + ".2"), Path(str(self.path) + ".1"), self.path]
        actual = b"".join(path.read_bytes() for path in parts if path.exists())
        self.assertEqual(actual, data)
        self.assertTrue(Path(str(self.path) + ".1").exists())
        self.assertTrue(all(path.stat().st_size <= 10 << 20 for path in parts if path.exists()))
        self.assertEqual(check_logs.tail(self.path, 1), ["last record"])

    def test_age_rotation_preserves_failed_steps_tail(self):
        self.path.parent.mkdir()
        self.path.write_text("error: the check refused\n")
        self.path.with_name("." + self.path.name + ".since").write_text(str(int(time.time()) - 2 * 86_400))
        with check_logs.bounded_log(self.root, self.path):
            pass
        self.assertEqual(self.path.read_bytes(), b"")
        self.assertEqual(check_logs.tail(self.path), ["error: the check refused"])

    def test_sink_failure_is_not_a_green_check(self):
        self.path.mkdir(parents=True)
        with self.assertRaisesRegex(RuntimeError, "sink failed"):
            with check_logs.bounded_log(self.root, self.path):
                pass

    def test_an_exception_still_closes_and_reaps_the_owned_sink(self):
        processes = []
        start = check_logs.subprocess.Popen
        def owned(*args, **kwargs):
            proc = start(*args, **kwargs)
            processes.append(proc)
            return proc
        with mock.patch.object(check_logs.subprocess, "Popen", owned):
            with self.assertRaisesRegex(ValueError, "check was canceled"):
                with check_logs.bounded_log(self.root, self.path) as out:
                    out.write(b"before cancellation\n")
                    raise ValueError("check was canceled")
        self.assertTrue(processes)
        self.assertTrue(all(proc.poll() is not None for proc in processes))
        self.assertIn("before cancellation", self.path.read_text())
