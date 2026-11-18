# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""`make install` in the install turn (spec AC-104), on a real repository in a temporary folder: two installs'
staging at once, one by hand beside a landing's install, and a landing's own make install inside its turn."""
import os
import select
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
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

    def spawn(self, *command, env=None):
        """`install_turn.py <command>` in the primary checkout, ended at the test's end if still running."""
        run = subprocess.Popen([sys.executable, str(SCRIPTS / "install_turn.py"), *command], cwd=self.repo, text=True,
                               stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=dict(os.environ, **(env or {})))
        self.addCleanup(lambda: run.poll() is None and (run.kill(), run.communicate()))
        return run

    def installs(self, *names, env=None):
        """Each name's staging, all started at once: [(exit status, stdout, stderr)]."""
        runs = [self.spawn(sys.executable, "-c", STAGE, str(self.out), name, env=env) for name in names]
        ended = []
        for run in runs:
            out, err = run.communicate(timeout=30)
            ended.append((run.returncode, out, err))
        return ended

    def test_two_installs_staging_at_once_take_turns(self):  # AC-104
        ended = self.installs("one", "two")
        self.assertEqual([code for code, *_ in ended], [0, 0], ended)
        (_, first_end), (second_start, _) = sorted(tuple(map(float, (self.out / f"ran-{n}").read_text().split())) for n in ("one", "two"))
        self.assertLessEqual(first_end, second_start, "the second stages only after the first has ended")
        self.assertTrue((self.out / "package/terminum/agents/macos-aarch64").is_dir())

    def test_without_the_turn_they_collide_as_the_symptom(self):  # AC-104: what the turn prevents
        ended = self.installs("one", "two", env={install_turn.HELD: "1", "BARRIER": "1"})
        self.assertIn("FileExistsError", "".join(err for *_, err in ended), "the staging folder raced")

    def test_one_by_hand_waits_for_a_landings_install_and_says_whose(self):  # AC-104
        with Turn(self.repo, INSTALL_TURN) as held:
            held.take_as("a landing's After (pid 4242)", self.fail)
            run = self.spawn(sys.executable, "-c", "print('installed')")
            self.assertTrue(select.select([run.stdout], [], [], 10)[0], "it says why it waits")
            self.assertEqual(run.stdout.readline(), "make install: a landing's After (pid 4242) holds the install turn: waiting for its end\n")
            time.sleep(0.1)
            self.assertIsNone(run.poll(), "it waits while the turn is held")
        out, _ = run.communicate(timeout=10)
        self.assertEqual((run.returncode, out), (0, "installed\n"))

    def test_the_holders_own_make_install_runs_inside_its_turn(self):  # AC-104, AC-198
        with Turn(self.repo, INSTALL_TURN) as held:
            self.assertTrue(held.take())
            run = self.spawn(sys.executable, "-c", "print('ran')", env={install_turn.HELD: "1"})
            self.assertEqual((run.communicate(timeout=10)[0], run.returncode), ("ran\n", 0), "no wait for its own caller")
        with mock.patch.object(land.subprocess, "run") as made:
            land.make(self.repo, "install")
        self.assertEqual(made.call_args.kwargs["env"][install_turn.HELD], "1", "a landing's make install is in its turn")

    def test_a_landings_install_says_whose_install_it_waits_for(self):  # AC-104
        said, waits = [], threading.Event()
        land.install_mark(self.repo).write_text("4f2a91c07\n")
        with mock.patch.object(land, "landing_holder", return_value=None), mock.patch.object(land, "install_now") as now:
            with Turn(self.repo, INSTALL_TURN) as held:
                held.take_as("make install in /Users/alex/terminum (pid 7)", self.fail)
                pending = threading.Thread(target=land.install_pending, args=(self.repo,),
                                           kwargs={"say": lambda words: (said.append(words), waits.set())})
                pending.start()
                self.assertTrue(waits.wait(10))
                self.assertEqual(said, ["install: make install in /Users/alex/terminum (pid 7) holds the install turn: waiting for its end"])
                self.assertFalse(now.called, "nothing installed while the other holds the turn")
            pending.join(10)
            now.assert_called_once()

    def test_the_commands_status_is_its_own(self):  # AC-104
        run = self.spawn("false")
        run.communicate(timeout=10)
        self.assertEqual(run.returncode, 1)


if __name__ == "__main__":
    unittest.main()
