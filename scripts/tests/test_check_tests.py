# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A runner's checks split out into `*_checks.py` modules select it (spec AC-104); nothing is run."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("check_changed", Path(__file__).resolve().parents[1] / "check_changed.py")
check_changed = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_changed)
from testrun import short_tmp
REPO = Path(__file__).resolve().parents[2]
CASE = "7" + "77"   # spelled so this file cites no case
GUARD = 'if __name__ == "__main__":\n    main()\n'


class CheckModules(unittest.TestCase):
    def tree(self, tmp):
        scripts = Path(tmp) / "scripts"
        scripts.mkdir()
        (scripts / "e2e_runner.py").write_text(f"from first_checks import a  # noqa: E402\n{GUARD}")
        (scripts / "first_checks.py").write_text("import json\ndef a():\n    import os, second_checks\n")
        (scripts / "second_checks.py").write_text(f'check("AC-{CASE} a thing")\n')
        (scripts / "e2e_other.py").write_text(f"from e2e_ws import check\n{GUARD}")
        (scripts / "e2e_ws.py").write_text(f"# AC-{CASE}\n")
        return Path(tmp)

    def test_a_case_cited_in_an_imported_check_module_selects_its_runner(self):  # AC-104
        with tempfile.TemporaryDirectory(dir=short_tmp()) as tmp:
            root = self.tree(tmp)
            self.assertEqual(check_changed.citing({CASE}, root), ["scripts/e2e_runner.py"])
            self.assertEqual(check_changed.uncited({CASE}, root), [])
            got = dict(check_changed.plan(["terminumd/src/x.rs"], {CASE}, root=root))
            self.assertEqual(got["e2e_runner"], "python3 scripts/e2e_runner.py")
            self.assertNotIn("e2e_other", got, "a shared helper's citations stay its own (AC-104)")

    def test_a_changed_check_module_runs_its_runner_and_is_not_run(self):  # AC-104
        with tempfile.TemporaryDirectory(dir=short_tmp()) as tmp:
            root = self.tree(tmp)
            got = dict(check_changed.plan(["scripts/second_checks.py"], set(), root=root))
            self.assertEqual(got["e2e_runner"], "python3 scripts/e2e_runner.py")
            self.assertNotIn("second_checks", got)
            self.assertNotIn("e2e_other", got)
            self.assertNotIn("e2e_runner", dict(check_changed.plan(["scripts/e2e_ws.py"], set(), root=root)))

    def test_a_deleted_check_module_still_runs_the_runner_importing_it(self):  # AC-104
        with tempfile.TemporaryDirectory(dir=short_tmp()) as tmp:
            root = self.tree(tmp)
            (root / "scripts" / "second_checks.py").unlink()
            got = dict(check_changed.plan(["scripts/second_checks.py"], set(), root=root))
            self.assertEqual(got["e2e_runner"], "python3 scripts/e2e_runner.py")

    def test_the_role_drills_hold_checks_run_with_e2e_roles(self):  # AC-104, AC-245
        self.assertIn("scripts/e2e_roles.py", check_changed.citing({"245"}, REPO))
        self.assertIn("e2e_roles", dict(check_changed.plan(["scripts/role_hold_checks.py"], set(), root=REPO)))


if __name__ == "__main__":
    unittest.main()
