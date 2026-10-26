# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The Mac checks' runner script (scripts/ci_mac.sh) against a stub ssh and rsync: the runner's
lock is taken before its tree is replaced, so a second run never changes the files of one under
way, and the lock is given back when nothing ran."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "ci_mac.sh"
# ssh: its first command run in the stub runner's home (the run itself only logged); rsync: logged
SSH = """#!/bin/bash
for a; do last=$a; done
echo "ssh ${last//$'\n'/ }" >> "$LOG"
case $last in *"bash -lc"*) exit 0 ;; esac
cd "$RUNNER" && bash -c "$last"
"""
RSYNC = """#!/bin/bash
echo "rsync" >> "$LOG"
"""


class CiMacTest(unittest.TestCase):
    def run_script(self, root):
        env = {**os.environ, "PATH": f"{root / 'bin'}:{os.environ['PATH']}", "LOG": str(root / "log"),
               "RUNNER": str(root / "runner"), "TM_MAC_RUNNER": "mac-runner"}
        r = subprocess.run(["bash", str(SCRIPT)], env=env, capture_output=True, text=True, timeout=30)
        log = (root / "log").read_text().splitlines() if (root / "log").exists() else []
        (root / "log").unlink(missing_ok=True)
        return r, log

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "bin").mkdir()
        (self.root / "runner").mkdir()
        for name, text in (("ssh", SSH), ("rsync", RSYNC)):
            (self.root / "bin" / name).write_text(text)
            (self.root / "bin" / name).chmod(0o755)

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_run_under_way_keeps_its_tree(self):
        (self.root / "runner/ci/terminum.lock").mkdir(parents=True)
        r, log = self.run_script(self.root)
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertNotIn("rsync", log, "the tree of the run under way is left as it is")
        self.assertTrue((self.root / "runner/ci/terminum.lock").is_dir(), "and its lock too")

    def test_the_lock_comes_before_the_tree_and_the_run_gives_it_back(self):
        r, log = self.run_script(self.root)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual([x.split()[0] for x in log], ["ssh", "rsync", "ssh"], log)
        self.assertIn('trap "rmdir ~/ci/terminum.lock" EXIT', log[-1], "the run gives the lock back")
        self.assertTrue((self.root / "runner/ci/terminum.lock").is_dir(), "the stub ran nothing: still held")


if __name__ == "__main__":
    unittest.main()
