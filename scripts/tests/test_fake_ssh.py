# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The stand-in ssh of the remote checks (scripts/fake_ssh.py): its `-R` forward carries an HTTP
request from the "host" to a Mac-side socket and back, is bound before the command runs, stays on
disk after the exit as sshd leaves it, and is refused as sshd refuses one (terminum-remote.md AS-84);
its `-L` forward still works."""
import errno
import http.server
import os
import shutil
import socket
import socketserver
import stat
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "fake_ssh.py"
# the "host's" client: one GET over a Unix socket, the answer's body printed
GET = ("import socket,sys; s=socket.socket(socket.AF_UNIX); s.connect(sys.argv[1]); "
       "s.sendall(b'GET /hook HTTP/1.0\\r\\n\\r\\n'); d=b''\n"
       "while (c := s.recv(4096)): d += c\n"
       "print(d.split(b'\\r\\n\\r\\n', 1)[1].decode())")


class Hello(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        body = f"hello-from-mac {self.path}".encode()
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *_):
        pass


class FakeSshTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="fssh-t-", dir="/tmp")  # short: Unix socket paths ≤ 104 bytes
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)
        self.home = os.path.join(self.root, "home")
        os.makedirs(os.path.join(self.home, ".terminum", "run"), mode=0o700)
        self.remote = os.path.join(self.home, ".terminum", "run", "m-abc-12345678.sock")
        self.local = os.path.join(self.root, "l.sock")

    def serve(self, path):
        srv = socketserver.ThreadingUnixStreamServer(path, Hello)
        srv.daemon_threads = True
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.server_close)
        self.addCleanup(srv.shutdown)

    def ssh(self, *args, env=None):
        e = {k: v for k, v in os.environ.items() if not k.startswith("FAKE_SSH_")}
        e.update(FAKE_SSH_HOME=self.home, FAKE_SSH_TMUX_SOCKET="tm-test-fssh-unit", **(env or {}))
        return subprocess.run([sys.executable, str(SCRIPT), "-T", "-o", "BatchMode=yes", *args], env=e,
                              capture_output=True, text=True, timeout=30)

    def reverse(self, eofw="no", cmd=None, env=None):
        cmd = cmd or f"[ -S {self.remote} ] && echo SOCK-FIRST; {sys.executable} -I -c \"$GET\" {self.remote}"
        return self.ssh("-o", "ControlPath=none", "-o", f"ExitOnForwardFailure={eofw}",
                        "-R", f"{self.remote}:{self.local}", "devbox.example", cmd, env={"GET": GET, **(env or {})})

    def assert_left_socket(self):
        st = os.lstat(self.remote)
        self.assertTrue(stat.S_ISSOCK(st.st_mode))
        self.assertEqual(stat.S_IMODE(st.st_mode), 0o600)
        s = socket.socket(socket.AF_UNIX)
        with self.assertRaises(OSError) as e:
            s.connect(self.remote)
        s.close()
        self.assertEqual(e.exception.errno, errno.ECONNREFUSED)

    def test_reverse_round_trip_socket_first_and_left_behind(self):
        self.serve(self.local)
        r = self.reverse()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.split(), ["SOCK-FIRST", "hello-from-mac", "/hook"])
        self.assertEqual(r.stderr, "")
        self.assert_left_socket()  # sshd does not unlink it (RR1 b)
        # the stale file refuses the next forward (RR1 c): the warning, and the command still runs
        r = self.reverse(cmd=f"[ -S {self.remote} ] && echo STALE; echo RAN")
        self.assertEqual(r.returncode, 0)
        self.assertIn(f"Warning: remote port forwarding failed for listen path {self.remote}", r.stderr)
        self.assertEqual(r.stdout.split(), ["STALE", "RAN"])

    def test_refused_warns_and_runs_without_it(self):
        r = self.reverse(cmd=f"[ -S {self.remote} ] || echo NO-SOCK; echo RAN", env={"FAKE_SSH_REFUSE_R": "1"})
        self.assertEqual(r.returncode, 0)
        self.assertEqual(r.stderr, f"Warning: remote port forwarding failed for listen path {self.remote}\n")
        self.assertEqual(r.stdout.split(), ["NO-SOCK", "RAN"])
        self.assertFalse(os.path.lexists(self.remote))

    def test_refused_under_exit_on_forward_failure_ends_before_the_command(self):
        r = self.reverse(eofw="yes", cmd="echo RAN", env={"FAKE_SSH_REFUSE_R": "1"})
        self.assertEqual(r.returncode, 255)
        self.assertEqual(r.stderr, f"Error: remote port forwarding failed for listen path {self.remote}\n")
        self.assertEqual(r.stdout, "")

    def test_refused_silently_at_log_level_error(self):
        r = self.ssh("-o", "LogLevel=ERROR", "-R", f"{self.remote}:{self.local}", "devbox", "echo RAN",
                     env={"FAKE_SSH_REFUSE_R": "1"})
        self.assertEqual((r.returncode, r.stderr, r.stdout), (0, "", "RAN\n"))

    def test_refusal_only_for_its_destination(self):
        self.serve(self.local)
        r = self.reverse(env={"FAKE_SSH_REFUSE_R": "1", "FAKE_SSH_REFUSE_R_DEST": "other-host"})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("hello-from-mac", r.stdout)

    def test_missing_folder_is_refused(self):
        self.remote = os.path.join(self.home, "nodir", "b.sock")
        r = self.reverse(eofw="yes", cmd="echo RAN")
        self.assertEqual((r.returncode, r.stdout), (255, ""))
        self.assertIn(f"Error: remote port forwarding failed for listen path {self.remote}", r.stderr)
        self.assertFalse(os.path.exists(os.path.dirname(self.remote)))  # sshd makes no folder

    def test_bare_local_name_is_a_bad_specification(self):
        r = self.ssh("-R", f"{self.remote}:l.sock", "devbox", "echo RAN")
        self.assertEqual((r.returncode, r.stdout), (255, ""))
        self.assertIn("Bad remote forwarding specification", r.stderr)

    def test_local_forward_still_joins(self):
        host_sock = os.path.join(self.root, "h.sock")
        self.serve(host_sock)
        r = self.ssh("-o", "ExitOnForwardFailure=yes", "-L", f"{self.local}:{host_sock}", "devbox",
                     f"{sys.executable} -I -c \"$GET\" {self.local}", env={"GET": GET})
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.split(), ["hello-from-mac", "/hook"])
        self.assertFalse(os.path.exists(self.local))  # its own -L socket is removed at exit, as before


if __name__ == "__main__":
    unittest.main()
