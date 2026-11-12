# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A run's tmux servers end with it (spec AC-102), a server whose socket file went included; what a
private terminumd started ends before its folder goes, one in a session of its own included."""
import importlib.util
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import unittest

SPEC = importlib.util.spec_from_file_location("testrun", Path(__file__).resolve().parents[1] / "testrun.py")
testrun = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(testrun)


class Servers(unittest.TestCase):
    def test_only_this_runs_sockets_match(self):
        ps = """7 /opt/homebrew/bin/tmux -L tm-test-remote-41 -f /dev/null -u -C new-session -A
8 tmux -S /tmp/x/tmux-501/tm-test-remote-41 new-session -d
9 tmux -L tm-test-remote-411 new-session -d
10 tmux -L terminum new-session -d
11 /usr/bin/ssh devbox.example tmux -L other
12 vim tm-test-remote-41
"""
        self.assertEqual(testrun.own_server_pids(ps, {"tm-test-remote-41"}), [7, 8])

    @unittest.skipUnless(shutil.which("tmux"), "no tmux")
    def test_a_server_whose_socket_went_is_ended(self):
        with tempfile.TemporaryDirectory(prefix="tt", dir="/tmp") as home:
            name = f"tm-test-testrun-{os.getpid()}"
            env = dict(os.environ, TMUX_TMPDIR=home)
            subprocess.run(["tmux", "-L", name, "-f", "/dev/null", "new-session", "-d", "sleep 60"], env=env, check=True)
            socket_file = Path(home) / f"tmux-{os.getuid()}" / name
            self.assertTrue(socket_file.exists())
            socket_file.unlink()  # what a dying predecessor does
            self.assertEqual(subprocess.run(["tmux", "-L", name, "kill-server"], env=env, capture_output=True).returncode, 1,
                             "kill-server cannot reach it")
            listed = lambda: testrun.own_server_pids(subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout, {name})
            self.assertEqual(len(listed()), 1)
            before = set(testrun._sockets)
            testrun._sockets.add(name)
            try:
                testrun.end_own_servers()
                for _ in range(100):
                    if not listed():
                        break
                    time.sleep(0.05)
                self.assertEqual(listed(), [])
            finally:
                testrun._sockets.clear()
                testrun._sockets.update(before)


    @unittest.skipUnless(shutil.which("tmux"), "no tmux")
    def test_end_tmux_ends_a_server_whose_socket_went(self):
        spec = importlib.util.spec_from_file_location("e2e_ws", Path(__file__).resolve().parents[1] / "e2e_ws.py")
        e2e_ws = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(e2e_ws)
        with tempfile.TemporaryDirectory(prefix="tt", dir=testrun.short_tmp()) as home:
            name = f"tm-test-endtmux-{os.getpid()}"
            saved = os.environ.get("TMUX_TMPDIR")
            os.environ["TMUX_TMPDIR"] = home
            try:
                subprocess.run(["tmux", "-L", name, "-f", "/dev/null", "new-session", "-d", "sleep 60"], check=True)
                (Path(home) / f"tmux-{os.getuid()}" / name).unlink()  # what a dying predecessor does
                listed = lambda: testrun.own_server_pids(subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True).stdout, {name})  # noqa: E731
                self.assertEqual(len(listed()), 1)
                e2e_ws.end_tmux(name)
                end = time.monotonic() + 5
                while listed() and time.monotonic() < end:
                    time.sleep(0.05)
                self.assertEqual(listed(), [], "ended by its pid where kill-server reached nothing")
            finally:
                for pid in listed():
                    os.kill(pid, 9)
                os.environ.pop("TMUX_TMPDIR") if saved is None else os.environ.update(TMUX_TMPDIR=saved)

# The child keeps a fake HOME but leaves both the parent's group and the private cwd.
ORPHAN = ("import os, subprocess, sys, time; sys.path.insert(0, sys.argv[3]); import owned_children; "
          "child = subprocess.Popen([sys.executable, '-c', "
          "'import os, time\\nwhile os.getppid() != 1: time.sleep(0.05)\\n' + sys.argv[1], sys.argv[2]], "
          "env={**os.environ, 'HOME': sys.argv[2]}, cwd='/', start_new_session=True); "
          "owned_children.register(owned_children.path(sys.argv[2]), child.pid); time.sleep(60)")


class WithItsFolder(unittest.TestCase):
    def orphan(self, code, folder):
        """The orphan's parent ended, as terminumd drops a host operation's ssh."""
        root = subprocess.Popen([sys.executable, "-c", ORPHAN, code, folder, str(Path(__file__).resolve().parents[1])])
        self.addCleanup(lambda: (root.kill(), root.wait()))
        end = time.monotonic() + 5
        while not testrun.naming(folder) and time.monotonic() < end:
            time.sleep(0.02)
        self.assertTrue(testrun.naming(folder), "the orphan was never seen")
        root.kill()
        root.wait()

    def test_an_orphan_writing_into_the_folder_late_is_waited_for(self):
        with tempfile.TemporaryDirectory(prefix="ts", dir=testrun.short_tmp()) as folder:
            self.orphan("import os, sys, time; time.sleep(1.5); os.makedirs(sys.argv[1] + '/remote'); open(sys.argv[1] + '/remote/.claude.json', 'w')", folder)
            self.assertEqual(testrun.end_naming(folder, settle=10), [], "it ended by itself, no signal")
            self.assertEqual(os.listdir(folder), ["remote"], "its write came before end_naming returned")
            self.assertEqual(testrun.naming(folder), [])

    def test_one_that_does_not_end_is_ended_by_its_pid(self):
        with tempfile.TemporaryDirectory(prefix="ts", dir=testrun.short_tmp()) as folder:
            self.orphan("import signal, time; signal.signal(signal.SIGTERM, signal.SIG_IGN); time.sleep(60)", folder)
            left = testrun.end_naming(folder, settle=0.3)
            self.assertEqual(len(left), 1, left)
            self.assertNotIn("HOME=", left[0], "its command, never its environment")
            self.assertEqual(testrun.naming(folder), [], "SIGKILL ended what ignored SIGTERM")

    def test_a_folder_beside_it_with_the_same_start_is_not_its(self):
        with tempfile.TemporaryDirectory(prefix="ts", dir=testrun.short_tmp()) as folder:
            sibling = folder + "x"
            os.mkdir(sibling)
            self.addCleanup(lambda: os.rmdir(sibling))
            child = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)", folder], cwd=sibling)
            self.addCleanup(lambda: (child.kill(), child.wait()))
            time.sleep(0.3)
            self.assertNotIn(child.pid, testrun.naming(folder), "mentioning the folder in argv proves no ownership")
            self.assertIn(child.pid, testrun.naming(sibling))


if __name__ == "__main__":
    unittest.main()
