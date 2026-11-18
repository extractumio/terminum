# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""check_signoff.py: an outside pull request's commits carry their authors' Signed-off-by (CONTRIBUTING.md)."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("check_signoff", Path(__file__).resolve().parents[1] / "check_signoff.py")
check_signoff = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_signoff)


def commit(message, email="alex@example.com", sha="a" * 40):
    return {"sha": sha, "email": email, "message": message}


class SignedOff(unittest.TestCase):
    def test_the_authors_own_trailer_counts(self):
        self.assertTrue(check_signoff.signed_off("Fix it\n\nSigned-off-by: Alex Doe <alex@example.com>\n", "alex@example.com"))
        self.assertTrue(check_signoff.signed_off("Fix it\n\nSigned-off-by: Alex Doe <Alex@Example.com>", "alex@example.com"), "case aside")

    def test_no_trailer_or_another_persons_does_not(self):
        self.assertFalse(check_signoff.signed_off("Fix it\n\nCo-Authored-By: Bot <bot@example.com>", "alex@example.com"))
        self.assertFalse(check_signoff.signed_off("Fix it\n\nSigned-off-by: Sam <sam@example.com>", "alex@example.com"), "someone else's sign-off")
        self.assertFalse(check_signoff.signed_off("Fix it, Signed-off-by: Alex <alex@example.com> inline", "alex@example.com"), "only a trailer line")
        self.assertFalse(check_signoff.signed_off("Signed-off-by: Alex <alex@example.com>", ""), "an author without an address")

    def test_unsigned_names_each_commit_lacking_it(self):
        good = commit("x\n\nSigned-off-by: Alex <alex@example.com>", sha="1" * 40)
        bad = commit("y", sha="2" * 40)
        self.assertEqual(check_signoff.unsigned([good, bad]), [bad])


class Range(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = self.tmp.name
        git = lambda *a: subprocess.run(["git", "-C", self.dir, *a], check=True, capture_output=True, text=True)
        git("init", "-q")
        git("config", "user.name", "Alex Doe")
        git("config", "user.email", "alex@example.com")
        git("config", "commit.gpgsign", "false")
        git("commit", "-q", "--allow-empty", "--no-verify", "-m", "base")
        git("commit", "-q", "--allow-empty", "--no-verify", "-s", "-m", "signed")
        git("commit", "-q", "--allow-empty", "--no-verify", "-m", "not signed")
        self.git = git

    def tearDown(self):
        self.tmp.cleanup()

    def test_a_local_range_is_read_and_judged(self):
        import os
        here = os.getcwd()
        os.chdir(self.dir)
        try:
            commits = check_signoff.from_range("HEAD~2..HEAD")
            self.assertEqual(len(commits), 2)
            self.assertEqual([c["message"].splitlines()[0] for c in check_signoff.unsigned(commits)], ["not signed"])
            self.assertEqual(check_signoff.main(["--range", "HEAD~2..HEAD"]), 1)
            self.assertEqual(check_signoff.main(["--range", "HEAD~2..HEAD~1"]), 0)
            self.assertEqual(check_signoff.main(["--range", "HEAD..HEAD"]), 2, "nothing to check is not a pass")
            self.assertEqual(check_signoff.main(["--range", "nosuch..HEAD"]), 2)
        finally:
            os.chdir(here)


if __name__ == "__main__":
    unittest.main()
