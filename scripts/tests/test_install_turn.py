# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""`make install` in the install turn (spec AC-104), on a real repository in a temporary folder: two installs'
staging at once, one by hand beside a landing's install, and a landing's own make install inside its turn."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest
from unittest import mock

SCRIPTS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPTS))
import install_turn  # noqa: E402
import land  # noqa: E402
from testrun import short_tmp  # noqa: E402
from worktree import INSTALL_TURN, Turn  # noqa: E402

# what package.py's stage does to the shared folder, slowed down: its folder emptied, then made again with
# mkdir(parents=True); with BARRIER both have emptied it before either makes it, so that two at once collide for sure
STAGE = """
import os, shutil, sys, time
from pathlib import Path
out, me, barrier = Path(sys.argv[1]), sys.argv[2], os.environ.get("BARRIER")
def meet(step):
    (out / f"{step}-{me}").touch()
    while barrier and len(list(out.glob(f"{step}-*"))) < 2:
        time.sleep(0.01)
pkg = out / "package" / "terminum"
meet("ready")
start = time.time()
shutil.rmtree(pkg, ignore_errors=True)
meet("emptied")
pkg.mkdir(parents=True)
time.sleep(0.3)
(pkg / "agents" / "macos-aarch64").mkdir(parents=True)
(out / f"ran-{me}").write_text(f"{start} {time.time()}")
"""


class InstallTurn(unittest.TestCase):
    def setUp(self):
        home = tempfile.TemporaryDirectory(dir=short_tmp())
        self.addCleanup(home.cleanup)
        self.repo = Path(home.name) / "primary"
        subprocess.run(["git", "init", "-q", str(self.repo)], check=True)
        self.out = Path(home.name) / "dist"
        self.out.mkdir()

    def installs(self, *names, env=None):
        """Each name's staging, under install_turn.py, all started at once."""
        runs = [subprocess.Popen([sys.executable, str(SCRIPTS / "install_turn.py"), sys.executable, "-c", STAGE, str(self.out), name],
                                 cwd=self.repo, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
                                 env=dict(os.environ, **(env or {})))
                for name in names]
        for run in runs:
            self.addCleanup(lambda r=run: r.poll() is None and (r.kill(), r.wait()))
        return [(run.wait(timeout=30), *run.communicate()) for run in runs]

    def test_two_installs_staging_at_once_take_turns(self):  # AC-104
        ended = self.installs("one", "two")
        self.assertEqual([code for code, *_ in ended], [0, 0], ended)
        (a0, a1), (b0, b1) = sorted(tuple(map(float, (self.out / f"ran-{n}").read_text().split())) for n in ("one", "two"))
        self.assertLessEqual(a1, b0, "the second stages only after the first has ended")
        self.assertTrue((self.out / "package/terminum/agents/macos-aarch64").is_dir())

    def test_without_the_turn_they_collide_as_the_symptom(self):  # AC-104: what the turn prevents
        ended = self.installs("one", "two", env={install_turn.HELD: "1", "BARRIER": "1"})
        self.assertIn("FileExistsError", "".join(err for _, _, err in ended), "the staging folder raced")

    def test_one_by_hand_waits_for_a_landings_install_and_says_whose(self):  # AC-104
        with Turn(self.repo, INSTALL_TURN) as held:
            self.assertTrue(held.take())
            held.hold_as("a landing's After")
            run = subprocess.Popen([sys.executable, str(SCRIPTS / "install_turn.py"), sys.executable, "-c", "print('installed')"],
                                   cwd=self.repo, stdout=subprocess.PIPE, text=True)
            self.addCleanup(lambda: run.poll() is None and (run.kill(), run.wait()))
            self.addCleanup(run.stdout.close)
            self.assertEqual(run.stdout.readline().strip(), "make install: a landing's After holds the install turn: waiting for its end")
            time.sleep(0.3)
            self.assertIsNone(run.poll(), "it waits while the turn is held")
        self.assertEqual(run.wait(timeout=10), 0)
        self.assertEqual(run.stdout.read().strip(), "installed")

    def test_the_holders_own_make_install_runs_inside_its_turn(self):  # AC-104, AC-198
        with Turn(self.repo, INSTALL_TURN) as held:
            self.assertTrue(held.take())
            done = subprocess.run([sys.executable, str(SCRIPTS / "install_turn.py"), sys.executable, "-c", "print('ran')"],
                                  cwd=self.repo, capture_output=True, text=True, timeout=10,
                                  env=dict(os.environ, **{install_turn.HELD: "1"}))
        self.assertEqual((done.returncode, done.stdout), (0, "ran\n"), "no wait for its own caller")
        with mock.patch.object(land.subprocess, "run") as run:
            land.make(self.repo, "install")
        self.assertEqual(run.call_args.kwargs["env"][install_turn.HELD], "1", "a landing's make install is in its turn")

    def test_the_commands_status_is_its_own(self):  # AC-104
        failed = subprocess.run([sys.executable, str(SCRIPTS / "install_turn.py"), "false"], cwd=self.repo, timeout=10)
        self.assertEqual(failed.returncode, 1)


if __name__ == "__main__":
    unittest.main()
