# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The leftovers' rules (spec AC-102): what counts, and above all what never does. Real sockets and folders in
a temporary folder; the one process ended is the one this test started."""
import importlib.util
import os
from pathlib import Path
import re
import shutil
import socket
import subprocess
import tempfile
import time
import unittest

SPEC = importlib.util.spec_from_file_location("leftovers", Path(__file__).resolve().parents[1] / "leftovers.py")
leftovers = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(leftovers)

BINARY = "/work/terminum/terminumd/target/debug/deps/terminumd-18e312ae2b0e93f4"
HELPER = f"{BINARY} --exact cli::tests::fake_terminumd --nocapture"
OLD = time.time() - leftovers.OLD_S - 60


class Processes(unittest.TestCase):
    def test_etime(self):
        self.assertEqual(leftovers.age_s("00:07"), 7)
        self.assertEqual(leftovers.age_s("01:00:07"), 3607)
        self.assertEqual(leftovers.age_s("10-12:47:23"), 10 * 86400 + 12 * 3600 + 47 * 60 + 23)

    def test_the_old_orphaned_helper_is_ended(self):
        self.assertEqual(leftovers.strays(f"40838 1 10-12:47:23 {HELPER}\n"), [(40838, HELPER, True)])

    def test_another_orphaned_test_binary_is_only_named(self):
        self.assertEqual(leftovers.strays(f"7 1 1-00:00:00 {BINARY}\n"), [(7, BINARY, False)])

    def test_a_built_server_with_no_parent_is_only_named(self):
        command = "/work/terminum/terminumd/target/debug/terminumd serve"
        self.assertEqual(leftovers.strays(f"7 1 1-00:00:00 {command}\n"), [(7, command, False)])

    def test_what_is_never_touched(self):
        for line in [
            f"7 900 1-00:00:00 {HELPER}",                                   # a test run still owns it
            f"7 1 05:59:00 {HELPER}",                                       # young: another session's run
            "7 1 1-00:00:00 /Users/alex/.terminum/daemon/terminumd serve",  # the installed server
            f"7 1 1-00:00:00 /usr/bin/ssh devbox.example {HELPER}",
            f"7 1 1-00:00:00 /bin/sleep 100 {HELPER}",
            "7 1 1-00:00:00 /opt/app/terminumd-18e312ae2b0e93f4",
        ]:
            with self.subTest(line=line):
                self.assertEqual(leftovers.strays(line + "\n"), [])

    def test_end_stops_the_process_it_was_told_of_and_leaves_another(self):
        command = "/bin/sleep 60"
        child = subprocess.Popen(command.split())
        try:
            while subprocess.run(["ps", "-o", "command=", "-p", str(child.pid)], capture_output=True, text=True).stdout.strip() != command:
                time.sleep(0.01)  # until the child has become its own program
            self.assertTrue(leftovers.end(child.pid, "another command"))
            self.assertIsNone(child.poll(), "a pid that now runs something else is left alone")
            self.assertTrue(leftovers.end(child.pid, command))
            self.assertIsNotNone(child.wait(5))
        finally:
            child.kill()
            child.wait()


class LostServers(unittest.TestCase):
    """A test's tmux server whose socket went with its run's folder (TERM-912): ended by its pid."""
    SERVER = "tmux -L tm-test-live-33522 -f /tmp/tmc.indcv8il/tml-T9fNjm/h/tmux.conf new-session -d -s main"

    def test_only_an_old_test_server_with_no_parent_and_no_socket_is_ended(self):
        gone, there = "/tmp/tmc.gone-912/tmux-501/tm-test-live-33522", tempfile.mkdtemp(prefix="tm-lost-")
        self.addCleanup(os.rmdir, there)
        bound = [(7, "tmux", gone), (8, "tmux", there), (9, "tmux", gone), (9, "tmux", there)]
        listed = "".join(f"{pid} 1 1-00:00:00 {self.SERVER}\n" for pid in (7, 8, 9, 10))
        self.assertEqual(leftovers.lost_servers(listed, bound), [(7, self.SERVER, True), (10, self.SERVER, False)],
                         "a socket still there (8, or one of 9's) keeps it; an unknown socket (10) is only named")
        self.assertEqual(leftovers.lost_servers(listed, None), [(pid, self.SERVER, False) for pid in (7, 8, 9, 10)])

    def test_a_server_whose_socket_went_from_outside_a_test_folder_is_only_named(self):
        bound = [(7, "tmux", "/tmp/tmux-501/tm-mine")]   # a /tmp cleaner's doing: SIGUSR1 brings it back
        self.assertEqual(leftovers.lost_servers(f"7 1 1-00:00:00 {self.SERVER}\n", bound), [(7, self.SERVER, False)])

    def test_what_is_never_ended(self):
        bound = [(7, "tmux", "/tmp/tmc.gone-912/tmux-501/x")]
        for line in [
            f"7 900 1-00:00:00 {self.SERVER}",                                  # a test run still owns it
            f"7 1 05:59:00 {self.SERVER}",                                      # young: another session's run
            "7 1 1-00:00:00 tmux -L terminum new-session -d -s main",           # the user's own server
            "7 1 1-00:00:00 /usr/bin/ssh devbox.example tmux -L tm-test-live-1 attach",
        ]:
            with self.subTest(line=line):
                self.assertEqual(leftovers.lost_servers(line + "\n", bound), [])

    def test_a_real_server_whose_folder_went_is_found_and_ended(self):
        folder = tempfile.mkdtemp(prefix="tm-lost-")
        env = {**os.environ, "TMUX_TMPDIR": folder}
        name = f"tm-test-lost-{os.getpid()}"
        subprocess.run(["tmux", "-f", "/dev/null", "-L", name, "new-session", "-d", "-s", "main", "-c", "/", "sleep 300"], env=env, check=True, timeout=10)
        pid = int(subprocess.run(["tmux", "-L", name, "display", "-p", "#{pid}"], env=env, capture_output=True, text=True, timeout=10).stdout)
        command = subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
        self.addCleanup(lambda: f"-L {name} " in subprocess.run(["ps", "-o", "command=", "-p", str(pid)], capture_output=True, text=True).stdout
                        and os.kill(pid, 9))
        old = f"{pid} 1 1-00:00:00 {command}\n"
        self.assertEqual(leftovers.lost_servers(old, leftovers.socket_paths()), [], "its socket is still there")
        shutil.rmtree(folder)
        self.assertEqual(leftovers.lost_servers(old, leftovers.socket_paths()), [(pid, command, True)])
        self.assertTrue(leftovers.end(pid, command))


class Sockets(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.TemporaryDirectory(prefix="lo-", dir="/tmp")
        self.addCleanup(self.home.cleanup)
        self.dir = Path(self.home.name)

    def socket_at(self, name, listening):
        server = socket.socket(socket.AF_UNIX)
        server.bind(str(self.dir / name))
        if listening:
            server.listen()
            self.addCleanup(server.close)
        else:
            server.close()
        return self.dir / name

    def test_dead_and_live_old_sockets_are_told_apart(self):
        self.socket_at("tm-dead", False)
        self.socket_at("tm-live", True)
        self.socket_at("terminum", False)  # the installed private server's name: not a test's
        later = time.time() + leftovers.OLD_S + 60
        dead, live = leftovers.sockets(self.dir, later)
        self.assertEqual([p.name for p in dead], ["tm-dead"])
        self.assertEqual([p.name for p in live], ["tm-live"])

    def test_a_young_socket_is_left(self):
        self.socket_at("tm-young", False)
        self.assertEqual(leftovers.sockets(self.dir, time.time()), ([], []))

    def test_no_socket_folder(self):
        self.assertEqual(leftovers.sockets(self.dir / "none", time.time()), ([], []))


class Folders(unittest.TestCase):
    def setUp(self):
        self.home = tempfile.TemporaryDirectory(prefix="lo-")
        self.addCleanup(self.home.cleanup)
        self.dir = Path(self.home.name)

    def folder(self, name, old=True):
        path = self.dir / name
        path.mkdir()
        if old:
            os.utime(path, (OLD, OLD))
        return path

    def test_old_test_folders_only(self):
        wanted = {self.folder("tm-settings-1091-add"), self.folder("tmh-73077-64"), self.folder("tmsq-2ebwXs"),
                  self.folder("fssh-5eali5_r"), self.folder(".fssh-5eali5_r")}
        self.folder("tm-young-1", old=False)
        for name in ["tmux-501", "tmate-501", "tmp-work", "tmp00004770", "terminum-audit-20261025", "TemporaryDirectory.x", "tm", "tm-", "other"]:
            self.folder(name)
        (self.dir / "tm-e2e-panel.png").write_bytes(b"not a folder")
        os.utime(self.dir / "tm-e2e-panel.png", (OLD, OLD))
        (self.dir / "tm-link").symlink_to(self.dir / "other")
        self.assertEqual(set(leftovers.folders([self.dir], time.time(), set())), wanted)

    def test_a_folder_with_recent_work_inside_stays(self):
        path = self.folder("tm-live-1")
        (path / "deep").mkdir()
        (path / "deep" / "log").write_text("written just now")
        os.utime(path / "deep", (OLD, OLD))
        os.utime(path, (OLD, OLD))
        self.assertEqual(leftovers.folders([self.dir], time.time(), set()), [])

    def test_a_folder_a_process_holds_open_stays(self):
        path = self.folder("tm-held-1")
        (path / "log").write_text("old")
        os.utime(path / "log", (OLD, OLD))
        os.utime(path, (OLD, OLD))
        self.assertEqual(leftovers.folders([self.dir], time.time(), set()), [path])
        self.assertEqual(leftovers.folders([self.dir], time.time(), {f"{path}/log"}), [])

    def test_no_temporary_folder(self):
        self.assertEqual(leftovers.folders([self.dir / "none"], time.time(), set()), [])


class TheListOfPrefixes(unittest.TestCase):
    # how tests outside scripts/tests make a folder at the top of a temporary folder
    MADE = re.compile(r"""(?:mkdtemp|TemporaryDirectory)\(prefix=["']([^"']+)|mktemp -d "?[^" )]*/([^/" )]*?)X{6}"""
                      r"""|mkdtempSync\(`\$\{shortTmp\(\)\}/([^`$]+)`|mkdtempSync\(path\.join\(os\.tmpdir\(\), "([^"]+)"|ownTmp\("([^"]+)\"""")

    def test_it_holds_every_prefix_a_test_uses(self):
        repo = Path(__file__).resolve().parents[2]   # no git: the Mac test runner's tree has none
        found = set()
        for folder in ("scripts", "web/test", "ui/test"):
            for path in (repo / folder).rglob("*"):
                if path.is_file() and path.suffix in (".py", ".sh", ".mjs") and "node_modules" not in path.parts and path.parent.name != "tests":
                    found |= {next(filter(None, m)) for m in self.MADE.findall(path.read_text(errors="replace"))}
        self.assertGreater(len(found), 10, found)  # a pattern that stopped matching checks nothing
        self.assertEqual([p for p in sorted(found) if not p.startswith(leftovers.FOLDER_PREFIXES)], [])


if __name__ == "__main__":
    unittest.main()
