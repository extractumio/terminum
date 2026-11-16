# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102: a macOS Python without os.waitid still pins the child until cleanup reaps it."""
import ctypes
import errno
from pathlib import Path
import signal
import subprocess
import sys
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_cancel


@unittest.skipUnless(sys.platform == "darwin", "Darwin's native waitid ABI")
class NativeChildStatus(unittest.TestCase):
    def wait_status(self, child):
        until = time.monotonic() + 5
        while time.monotonic() < until:
            result = check_cancel.unreaped_status(child)
            if result is not None:
                return result
            time.sleep(.01)
        self.fail("child did not become waitable")

    def test_exited_child_can_be_peeked_repeatedly_before_reaping(self):
        for status in (0, 7):
            with self.subTest(status=status), mock.patch.object(check_cancel.os, "waitid", None, create=True):
                child = subprocess.Popen(["/bin/sh", "-c", f"exit {status}"], start_new_session=True)
                try:
                    self.assertEqual(self.wait_status(child), status)
                    self.assertEqual(check_cancel.unreaped_status(child), status)
                    self.assertIsNone(child.returncode)
                    self.assertEqual(child.wait(timeout=5), status)
                finally:
                    if child.poll() is None:
                        child.kill()
                    child.wait()

    def test_running_and_signalled_child_keep_their_pid_until_reaped(self):
        with mock.patch.object(check_cancel.os, "waitid", None, create=True):
            child = subprocess.Popen(["/bin/sleep", "30"], start_new_session=True)
            try:
                self.assertIsNone(check_cancel.unreaped_status(child))
                child.send_signal(signal.SIGTERM)
                self.assertEqual(self.wait_status(child), -signal.SIGTERM)
                self.assertEqual(check_cancel.unreaped_status(child), -signal.SIGTERM)
                self.assertIsNone(child.returncode)
                self.assertEqual(child.wait(timeout=5), -signal.SIGTERM)
            finally:
                if child.poll() is None:
                    child.kill()
                child.wait()


class NativeErrors(unittest.TestCase):
    def test_interrupted_native_wait_is_retried(self):
        attempts = iter((errno.EINTR, 0))
        def invoke(*_):
            number = next(attempts)
            ctypes.set_errno(number)
            return -1 if number else 0
        wait = mock.Mock(side_effect=invoke)
        with mock.patch.object(check_cancel, "_darwin_waitid", return_value=wait):
            self.assertIsNone(check_cancel._darwin_info(42))
        self.assertEqual(wait.call_count, 2)

    def test_native_error_is_not_an_exit_status(self):
        def failed(*_):
            ctypes.set_errno(errno.ECHILD)
            return -1
        with mock.patch.object(check_cancel, "_darwin_waitid", return_value=failed):
            with self.assertRaises(ChildProcessError):
                check_cancel._darwin_info(42)

    def test_stopped_child_is_not_treated_as_a_completed_step(self):
        with mock.patch.object(check_cancel.sys, "platform", "darwin"), \
                mock.patch.object(check_cancel.os, "waitid", None, create=True), \
                mock.patch.object(check_cancel, "_darwin_info", return_value=mock.Mock(si_code=5, si_status=signal.SIGSTOP)):
            self.assertIsNone(check_cancel.unreaped_status(mock.Mock(pid=42)))
