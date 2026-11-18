# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""scripts/security_check.sh --message (the commit-msg hook): a commit message is judged by the
rules of the tree scan, git's comment lines and the diff of `commit -v` are not, and the
number of private patterns read is said, so an empty list shows."""
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "security_check.sh"
HOOK = ROOT / "scripts" / "hooks" / "commit-msg"
# an AWS key's shape, put together here so this file passes the tree scan itself
KEY = "AKIA" + "ABCDEFGHIJKLMNOP"


class MessageCheckTest(unittest.TestCase):
    def run_check(self, text, patterns=None, hook=False, author=None):
        with tempfile.TemporaryDirectory() as d:
            msg = Path(d) / "COMMIT_EDITMSG"
            msg.write_text(text)
            env = {k: v for k, v in os.environ.items() if k not in ("TM_PORT", "TM_APP_DIR")}
            env["XDG_CONFIG_HOME"] = d   # no default patterns file of this machine
            if author:   # the git author the hook keeps out of messages, in a config of the test's own
                (Path(d) / "gitconfig").write_text(f"[user]\n\tname = {author[0]}\n\temail = {author[1]}\n")
                env.update(GIT_CONFIG_GLOBAL=str(Path(d) / "gitconfig"), GIT_CONFIG_NOSYSTEM="1")
            if patterns is not None:
                (Path(d) / "pp").write_text(patterns)
                env["TM_PRIVATE_PATTERNS"] = str(Path(d) / "pp")
            else:
                env.pop("TM_PRIVATE_PATTERNS", None)
            cmd = [str(HOOK), str(msg)] if hook else ["bash", str(SCRIPT), "--message", str(msg)]
            return subprocess.run(cmd, cwd=ROOT, env=env, capture_output=True, text=True)

    def test_clean_message_passes(self):
        r = self.run_check("Fix the row menu\n\nThe menu closes on Esc.\n")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("ok   message: 1 files, 0 findings", r.stdout)
        self.assertIn("private patterns: none", r.stdout)

    def test_a_sign_off_may_name_its_author_the_body_may_not(self):
        sam = ("Sam Doe", "sam@example.com")
        r = self.run_check("Fix the row menu\n\nSigned-off-by: Sam Doe <sam@example.com>\n", author=sam)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        r = self.run_check("Fix the row menu, by Sam Doe\n\nSigned-off-by: Sam Doe <sam@example.com>\n", author=sam)
        self.assertNotEqual(r.returncode, 0, "the author's name outside the sign-off still fails")
        r = self.run_check(f"Fix\n\nSigned-off-by: Sam Doe <sam@example.com> {KEY}\n", author=sam)
        self.assertNotEqual(r.returncode, 0, "a secret in a sign-off line still fails")

    def test_secret_fails_and_is_not_printed(self):
        r = self.run_check(f"Fix\n\nkey {KEY}\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn(":3: AWS key", r.stdout)
        self.assertNotIn(KEY, r.stdout)

    def test_private_pattern_fails_and_is_counted(self):
        r = self.run_check("Runs on devbox-private now\n", patterns="# hosts\ndevbox-private\n")
        self.assertEqual(r.returncode, 1)
        self.assertIn("private patterns: 1", r.stdout)
        self.assertIn(":1: private pattern", r.stdout)

    def test_lines_with_a_hash_count_but_the_verbose_diff_does_not(self):
        # `commit -m`/`-F` keep lines that start with #: they are judged too
        r = self.run_check(f"Fix\n# runs with {KEY}\n")
        self.assertEqual(r.returncode, 1, r.stdout)
        text = f"Fix\n# ------------------------ >8 ------------------------\n+key {KEY}\n"
        r = self.run_check(text)
        self.assertEqual(r.returncode, 0, r.stdout)

    @unittest.skipUnless((Path(__file__).resolve().parents[2] / ".git").exists(), "no .git: the Mac test runner's tree has none")
    def test_hook_runs_the_message_check(self):
        r = self.run_check(f"Fix\n\nkey {KEY}\n", hook=True)
        self.assertEqual(r.returncode, 1)
        self.assertIn("AWS key", r.stdout)


class StagedCheckTest(unittest.TestCase):
    """--staged (the pre-commit hook) judges the index: a staged leak fails though the file on
    disk is clean again, and an unstaged or untracked one (another session's) does not."""

    def git(self, repo, *args):
        subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True)

    def test_judges_the_index_not_the_disk(self):
        with tempfile.TemporaryDirectory() as repo:
            (Path(repo) / "scripts").mkdir()
            (Path(repo) / "scripts" / "security_check.sh").write_bytes(SCRIPT.read_bytes())
            self.git(repo, "init", "-q")
            note = Path(repo) / "note.txt"
            note.write_text(f"key {KEY}\n")
            self.git(repo, "add", "note.txt")
            note.write_text("clean now\n")                       # disk clean, index not
            (Path(repo) / "other.txt").write_text(f"{KEY}\n")    # untracked: not this commit's
            env = {k: v for k, v in os.environ.items() if k not in ("TM_PORT", "TM_APP_DIR", "TM_PRIVATE_PATTERNS")}
            env["XDG_CONFIG_HOME"] = repo
            run = lambda: subprocess.run(["bash", "scripts/security_check.sh", "--staged"], cwd=repo, env=env,
                                         capture_output=True, text=True)
            r = run()
            self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
            self.assertIn("note.txt:1: AWS key", r.stdout)
            self.assertNotIn("other.txt", r.stdout)
            self.git(repo, "add", "note.txt")                      # the clean version staged
            r = run()
            self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
            self.assertIn("ok   staged: 1 files, 0 findings", r.stdout)


if __name__ == "__main__":
    unittest.main()
