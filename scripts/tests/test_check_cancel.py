# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102, AC-104: cancelling the real outer runner ends only its own guarded processes."""
import json
import contextlib
import io
import os
from pathlib import Path
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import check_cancel
import check_step
import owned_children
from testrun import short_tmp

SCRIPTS = Path(__file__).resolve().parents[1]


def wait_for(probe, seconds=75):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if result := probe():
            return result
        time.sleep(0.05)
    raise AssertionError("the recorded child never became ready")


def stop_recorded(child, orderly=False):
    if child.poll() is None:
        if orderly:
            child.send_signal(signal.SIGTERM)
            child.wait(timeout=150)  # includes the outer runner's bounded guard recovery
        else:
            child.kill()
    child.wait(timeout=10)


class OuterCancellation(unittest.TestCase):
    def exercise(self, number, *, recover=False, failed_guard=False, settling=False):
        with tempfile.TemporaryDirectory(prefix="tm-cancel-", dir=short_tmp()) as tmp:
            root = Path(tmp)
            (root / "scripts").symlink_to(SCRIPTS, target_is_directory=True)
            ready = root / "ready.json"
            later = root / "later"
            cooperative_ready = root / "cooperative-ready"
            cleaned = root / "cleaned"
            # Both children ignore cancellation; the detached one leaves both the group and cwd.
            ignore = "\nfor number in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT): signal.signal(number,signal.SIG_IGN)\n"
            detached = "import signal,time" + ignore + "time.sleep(180)"
            cooperative = ("import signal,time\nfrom pathlib import Path\nreceived=None\n"
                           "def receive(number,frame):\n global received\n received=received or number\n"
                           "for number in (signal.SIGTERM,signal.SIGHUP,signal.SIGINT): signal.signal(number,receive)\n"
                           f"Path({str(cooperative_ready)!r}).touch()\n"
                           "while received is None: time.sleep(0.01)\n"
                           f"time.sleep(5)\nPath({str(cleaned)!r}).write_text(str(received))\n")
            script = ("import json,os,signal,subprocess,sys,time\n"
                      f"sys.path.insert(0, {str(SCRIPTS)!r})\nimport owned_children\n"
                      f"{ignore}"
                      f"child=subprocess.Popen([sys.executable,'-c',{detached!r}], cwd='/', start_new_session=True)\n"
                      "owned_children.register(os.environ['TM_TEST_CHILDREN_FILE'],child.pid)\n"
                      f"cooperative=subprocess.Popen([sys.executable,'-c',{cooperative!r}])\n"
                      f"while not os.path.exists({str(cooperative_ready)!r}): time.sleep(0.01)\n"
                      "starts=owned_children.identities([os.getpid(),os.getppid(),child.pid,cooperative.pid])\n"
                      f"with open({str(ready)+'.pending'!r},'w') as out:\n"
                      " json.dump({'root':os.environ['TMPDIR'],'foreground':os.getpid(),'detached':child.pid,'cooperative':cooperative.pid,'guard':os.getppid(),'starts':starts},out)\n"
                      f"os.replace({str(ready)+'.pending'!r},{str(ready)!r})\n"
                      + ("sys.exit(0)\n" if settling else "while True: time.sleep(1)\n"))
            command = "exec " + shlex.join([sys.executable, "-I", "-c", script])
            harness = (f"import sys\nsys.path.insert(0,{str(SCRIPTS)!r})\n"
                       "from pathlib import Path\nimport check_changed,check_cancel\n"
                       f"root=Path({tmp!r})\ncheck_changed.REPO=root\n"
                       "check_changed.changed_files=lambda _: (['scripts/probe.py'],'HEAD')\n"
                       "check_changed.touched_cases=lambda *_: set()\ncheck_changed.caseless=lambda _: []\n"
                       "check_changed.git=lambda *_: ''\ncheck_changed.tree_id=lambda: 'cancel-probe'\n"
                       f"check_changed.plan=lambda *_: [('probe',{command!r}),('later',{('touch '+shlex.quote(str(later)))!r})]\n"
                       "run=check_changed.run\ncheck_changed.run=lambda steps: run(steps,root=root,gate=False)\n"
                       + ("check_cancel.CANCEL_WAIT_S=0.2\n" if recover else "") +
                       "sys.exit(check_changed.main([]))\n")
            info = None
            unrelated = subprocess.Popen(["sleep", "180"], start_new_session=True)
            outer = subprocess.Popen([sys.executable, "-I", "-c", harness], stdout=subprocess.PIPE,
                                     stderr=subprocess.STDOUT, text=True, start_new_session=True)
            try:
                info = wait_for(lambda: json.loads(ready.read_text()) if ready.exists() and ready.stat().st_size else None)
                # Wait until the detached child installed its signal handler too.
                time.sleep(0.2)
                if settling:
                    wait_for(lambda: owned_children.identity(info["foreground"]) is None)
                    time.sleep(14)  # cancellation late in the original settling interval needs a fresh grace period
                if recover:
                    os.kill(info["guard"], signal.SIGSTOP)
                if failed_guard:
                    os.kill(info["guard"], signal.SIGKILL)
                else:
                    outer.send_signal(number)
                    time.sleep(0.25)
                    outer.send_signal(signal.SIGINT)
                output, _ = outer.communicate(timeout=100)
                self.assertEqual(outer.returncode, 128 + (signal.SIGKILL if failed_guard else number), output)
                self.assertIsNone(unrelated.poll(), "an unrelated process was touched")
                for key in ("foreground", "detached", "cooperative", "guard"):
                    self.assertIsNone(owned_children.identity(info[key]), f"{key} still runs")
                self.assertFalse(Path(info["root"]).exists(), "private root survived cancellation")
                self.assertFalse(later.exists(), "a cancelled run started its next check")
                self.assertEqual(list((root / ".checks").glob("changed-*.ok")), [])
                if not recover and not failed_guard:
                    self.assertTrue(cleaned.exists(), "a shell descendant lost its five-second cleanup grace")
                    self.assertEqual(cleaned.read_text(), str(number), "the first signal must reach the whole group")
            finally:
                stop_recorded(outer, orderly=True)
                stop_recorded(unrelated)
                if info:
                    # Only PIDs explicitly published by our child; no pattern or environment scans.
                    pids = [info[key] for key in ("foreground", "detached", "cooperative", "guard")]
                    expected = {pid: info["starts"][str(pid)] for pid in pids}
                    check_step.end_pids(list(expected), expected)
                    if Path(info["root"]).exists():
                        import shutil
                        shutil.rmtree(info["root"])

    def test_cancel_outer_runner_with_stubborn_and_detached_children(self):
        for number in check_cancel.SIGNALS:
            with self.subTest(signal=number):
                self.exercise(number)

    def test_stuck_guard_is_recovered_after_cancellation_deadline(self):
        self.exercise(signal.SIGTERM, recover=True)

    def test_cancellation_during_settling_gives_descendants_a_fresh_grace_period(self):
        self.exercise(signal.SIGTERM, settling=True)

    def test_failed_guard_is_recovered_without_running_another_step(self):
        self.exercise(signal.SIGTERM, failed_guard=True)


class Boundaries(unittest.TestCase):
    def test_group_notification_happens_once_even_when_the_guard_receives_its_own_signal(self):
        with self.assertRaises(SystemExit) as ended:
            with check_step.GuardCancellation() as cancelled:
                def delivered(_group, number):
                    cancelled.receive(number, None)
                with mock.patch.object(os, "killpg", side_effect=delivered) as kill:
                    cancelled.receive(signal.SIGTERM, None)
                    self.assertTrue(cancelled.forward(42))
                    cancelled.receive(signal.SIGINT, None)
                    self.assertFalse(cancelled.forward(42))
                    kill.assert_called_once_with(42, signal.SIGTERM)
        self.assertEqual(ended.exception.code, 143)

    def test_failed_metadata_discovery_resumes_the_same_guard(self):
        with mock.patch.object(owned_children, "identity", return_value="original"), \
                mock.patch.object(check_step, "own_processes", side_effect=OSError("metadata query failed")), \
                mock.patch.object(os, "kill") as kill:
            with self.assertRaisesRegex(OSError, "metadata query failed"):
                check_cancel.recover(mock.Mock(pid=42), "original", "private")
        self.assertEqual(kill.call_args_list, [mock.call(42, signal.SIGSTOP), mock.call(42, signal.SIGCONT)])

    def test_signal_during_root_removal_cannot_return_a_pass(self):
        with tempfile.TemporaryDirectory(prefix="tm-cancel-final-", dir=short_tmp()) as tmp:
            folder = Path(tmp) / "private"
            folder.mkdir()
            def remove(_):
                signal.getsignal(signal.SIGTERM)(signal.SIGTERM, None)
            with mock.patch.object(check_cancel.tempfile, "mkdtemp", return_value=str(folder)), \
                    mock.patch.object(check_cancel.subprocess, "Popen", return_value=mock.Mock(pid=42)), \
                    mock.patch.object(owned_children, "identity", return_value="original"), \
                    mock.patch.object(os, "waitid", return_value=mock.Mock(si_code=os.CLD_EXITED, si_status=0)), \
                    mock.patch.object(check_cancel.shutil, "rmtree", side_effect=remove):
                with self.assertRaises(SystemExit) as ended:
                    check_cancel.run_guard([], [], cwd=tmp, stdout=None)
                self.assertEqual(ended.exception.code, 143)

    def test_failed_recovery_keeps_the_root_and_reports_failure(self):
        with tempfile.TemporaryDirectory(prefix="tm-cancel-fail-", dir=short_tmp()) as tmp:
            folder = Path(tmp) / "private"
            folder.mkdir()
            with mock.patch.object(check_cancel.tempfile, "mkdtemp", return_value=str(folder)), \
                    mock.patch.object(check_cancel.subprocess, "Popen", return_value=mock.Mock(pid=42)), \
                    mock.patch.object(owned_children, "identity", return_value="original"), \
                    mock.patch.object(os, "waitid", return_value=mock.Mock(si_code=os.CLD_EXITED, si_status=1)), \
                    mock.patch.object(check_cancel, "recover", side_effect=OSError("metadata query failed")), \
                    contextlib.redirect_stdout(io.StringIO()) as output:
                with self.assertRaisesRegex(OSError, "metadata query failed"):
                    check_cancel.run_guard([], [], cwd=tmp, stdout=None)
            self.assertTrue(folder.exists())
            self.assertIn("private root retained", output.getvalue())

    def test_reused_guard_group_is_not_selected_for_cleanup(self):
        child = mock.Mock(pid=42)
        with mock.patch.object(owned_children, "identity", return_value="reused"), \
                mock.patch.object(check_step, "own_processes", return_value=[]) as discover, \
                mock.patch.object(owned_children, "identities", return_value={}), \
                mock.patch.object(owned_children, "records", return_value={}), \
                mock.patch.object(check_step, "end_pids") as end, mock.patch.object(os, "kill") as kill:
            check_cancel.recover(child, "original", "private")
        self.assertEqual(discover.call_args_list, [mock.call("private", None), mock.call("private", None)])
        end.assert_called_once_with([], {})
        kill.assert_not_called()

    def test_reused_guard_pid_is_never_signalled(self):
        with mock.patch.object(owned_children, "identity", return_value="reused"), mock.patch.object(os, "kill") as kill:
            check_cancel.signal_guard(mock.Mock(pid=42), "original", signal.SIGKILL)
        kill.assert_not_called()

    def test_failed_identity_query_is_not_treated_as_an_ended_guard(self):
        with mock.patch.object(owned_children, "identity", side_effect=OSError("metadata query failed")), mock.patch.object(os, "kill") as kill:
            with self.assertRaisesRegex(OSError, "metadata query failed"):
                check_cancel.signal_guard(mock.Mock(pid=42), "original", signal.SIGTERM)
        kill.assert_not_called()

    def test_handlers_record_first_signal_and_restore_the_previous_handlers(self):
        previous = {number: signal.getsignal(number) for number in check_cancel.SIGNALS}
        with self.assertRaises(SystemExit) as ended:
            with check_cancel.Cancellation() as cancelled:
                cancelled.receive(signal.SIGHUP, None)
                cancelled.receive(signal.SIGINT, None)
                self.assertEqual(cancelled.number, signal.SIGHUP)
        self.assertEqual(ended.exception.code, 129)
        self.assertEqual({number: signal.getsignal(number) for number in check_cancel.SIGNALS}, previous)


if __name__ == "__main__":
    unittest.main()
