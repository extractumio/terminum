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

    def test_retry_preserves_all_segments_and_starts_an_independent_log(self):  # AC-196, AC-197
        data = b"first attempt output\n" * 600_000 + b"first attempt failed\n"
        with check_logs.bounded_log(self.root, self.path) as out:
            out.write(data)
        first = self.path.with_name("changed.test.first.log")
        Path(str(first) + ".2").write_bytes(b"stale attempt\n")
        check_logs.preserve_attempt(self.path, first)
        parts = [Path(str(first) + suffix) for suffix in (".2", ".1", "")]
        self.assertEqual(b"".join(p.read_bytes() for p in parts if p.exists()), data)
        self.assertFalse(self.path.with_name("." + self.path.name + ".since").exists())
        with check_logs.bounded_log(self.root, self.path) as out:
            out.write(b"retry passed\n")
        self.assertEqual(check_logs.tail(self.path), ["retry passed"])
        self.assertEqual(check_logs.tail(first, 1), ["first attempt failed"])

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

    def test_cancellation_result_survives_an_inherited_output_pipe(self):
        holder = None
        try:
            with mock.patch.object(check_logs, "DRAIN_SECONDS", 0.1):
                with self.assertRaisesRegex(ValueError, "canceled"):
                    with check_logs.bounded_log(self.root, self.path) as out:
                        holder = check_logs.subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"], stdout=out)
                        out.write(b"before cancellation\n")
                        raise ValueError("canceled")
        finally:
            if holder is not None:
                holder.terminate()
                holder.wait(timeout=10)

    def test_handled_callers_exception_does_not_hide_a_sink_failure(self):
        self.path.mkdir(parents=True)
        try:
            raise ValueError("already handled by caller")
        except ValueError:
            with self.assertRaisesRegex(RuntimeError, "sink failed"):
                with check_logs.bounded_log(self.root, self.path):
                    pass
