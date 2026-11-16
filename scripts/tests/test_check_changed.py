# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Which checks a change gets (spec AC-104, AC-146); nothing is run."""
from contextlib import redirect_stdout
import importlib.util
import io
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("check_changed", Path(__file__).resolve().parents[1] / "check_changed.py")
check_changed = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(check_changed)
from testrun import short_tmp
from test_land import git
REPO = Path(__file__).resolve().parents[2]


def steps(files, cited=()):
    return dict(check_changed.plan(files, set(), root=REPO, tests_of_cases=list(cited)))


class Plan(unittest.TestCase):
    def test_docs_alone_build_nothing_and_their_check_blocks_nothing(self):
        got = steps(["docs/USAGE.md", "docs/specs/terminum.md"])
        self.assertEqual(list(got), ["security", "docs"])
        self.assertTrue(got["docs"].startswith("-"))
        self.assertFalse(got["security"].startswith("-"), "a public repository: the scan blocks")

    def test_a_rust_change_builds_lints_tests_its_module_and_the_critical_ones(self):
        got = steps(["terminumd/src/hosts/tunnel.rs"])
        self.assertNotIn("build", got, "no release build in a tree: the primary checkout's install builds it")
        self.assertIn("debug-build", got)
        self.assertIn("clippy", got)
        self.assertIn("hosts::tunnel", got["cargo"])
        for critical in check_changed.CRITICAL_CARGO:
            self.assertIn(critical, got["cargo"])
        self.assertIn("critical", got)

    def test_a_rust_source_naming_cases_at_its_top_brings_the_e2e_scripts_citing_them(self):  # AC-104
        # no changed lines: the cases come from the file's top alone (and the Mac runner's tree has no .git)
        with mock.patch.object(check_changed, "git", return_value=""):
            cases = check_changed.touched_cases(["terminumd/src/tmux/server_local.rs"], "HEAD", cwd=REPO)
        self.assertLessEqual({"01", "89"}, cases)
        self.assertIn("scripts/e2e_attach.py", check_changed.citing(cases, REPO))

    def test_a_rust_source_naming_no_case_is_said(self):  # AC-104
        with tempfile.TemporaryDirectory() as root:
            src = Path(root, "terminumd/src")
            src.mkdir(parents=True)
            (src / "plain.rs").write_text("//! A helper.\n" + "\n" * 6 + "// AC-07 too far down to count\n")
            (src / "named.rs").write_text("//! Sizing (AC-13).\n")
            (src / "plain_tests.rs").write_text("")
            files = ["terminumd/src/plain.rs", "terminumd/src/named.rs", "terminumd/src/plain_tests.rs", "terminumd/src/cli/tests/mod.rs", "terminumd/src/gone.rs", "web/static/a.js"]
            self.assertEqual(check_changed.caseless(files, Path(root)), ["terminumd/src/plain.rs"])

    def test_the_module_of_a_file(self):
        self.assertEqual(check_changed.module_of("terminumd/src/cli/install_tests.rs"), "cli::install")
        self.assertEqual(check_changed.module_of("terminumd/src/cli/tests/mod.rs"), "cli::tests")
        self.assertEqual(check_changed.module_of("terminumd/src/agentmarks.rs"), "agentmarks")
        self.assertIsNone(check_changed.module_of("terminumd/src/main.rs"))

    def test_the_tests_citing_a_touched_case_run(self):
        got = steps(["terminumd/src/agentmarks.rs"], ["web/test/e2e_marks.mjs", "scripts/e2e_hooks.py", "terminumd/src/hosts/link_tests.rs"])
        self.assertIn("webMarks", got["page"])
        self.assertEqual(got["e2e_hooks"], "python3 scripts/e2e_hooks.py")
        self.assertIn("hosts::link", got["cargo"])

    def test_a_page_change_runs_what_shows_it(self):
        got = steps(["web/static/banners.js"], ["web/test/e2e_screen.mjs"])
        self.assertIn("page-unit", got)
        self.assertIn("webScreen", got["page"])
        self.assertIn("critical", got)
        self.assertNotIn("clippy", got)

    def test_a_panel_change_runs_the_panel(self):
        got = steps(["ui/src/tree.ts"])
        self.assertIn("ui-unit", got)
        self.assertIn("panel", got)

    def test_a_script_runs_the_scripts_tests_only(self):
        got = steps(["scripts/worktree.py"])
        self.assertIn("scripts", got)
        self.assertNotIn("ui", got)
        self.assertNotIn("critical", got)
        self.assertNotIn("cargo", got)

    def test_dockers_checks_take_arguments_and_are_run_by_hand(self):
        self.assertNotIn("e2e_remote_sshd", steps(["terminumd/src/hosts/link.rs"], ["scripts/e2e_remote_sshd.py"]))
        self.assertNotIn("e2e_remote_sshd", steps(["scripts/e2e_remote_sshd.py"]))

    def test_a_live_part_runs_with_the_critical_step(self):
        got = steps(["scripts/worktree.py"], ["web/test/e2e_livenotes.mjs"])
        self.assertEqual(got["critical"], "node web/test/e2e_live.mjs chromium")
        self.assertIn("debug-build", got)

    def test_an_e2e_script_has_the_debug_build_it_runs(self):
        got = steps(["terminumd/src/x.rs"], ["scripts/e2e_hooks.py"])
        self.assertLess(list(got).index("debug-build"), list(got).index("e2e_hooks"))

    def test_ac102_selects_real_runners_without_the_imported_helper(self):  # AC-102, AC-104
        got = dict(check_changed.plan(["scripts/e2e_ws.py"], {"102"}, root=REPO))
        self.assertNotIn("e2e_ws", got)
        for name in ("e2e_attach", "e2e_restore"):
            self.assertEqual(got[name], f"python3 scripts/{name}.py")
        self.assertIn("scripts", got)
        self.assertIn("critical", got)

    def test_a_changed_or_explicitly_cited_helper_is_not_executed(self):  # AC-104
        for files, cited in ((["scripts/e2e_ws.py"], []), (["scripts/worktree.py"], ["scripts/e2e_ws.py"])):
            with self.subTest(files=files, cited=cited):
                self.assertEqual(list(steps(files, cited)), ["security", "scripts"])

    def test_helper_citations_and_guard_text_are_not_test_coverage(self):  # AC-104
        with tempfile.TemporaryDirectory(dir=short_tmp()) as tmp:
            root = Path(tmp)
            scripts = root / "scripts"
            scripts.mkdir()
            case = "7" + "77"  # no real case citation in this test's own source
            (scripts / "e2e_helper.py").write_text(f'''"""AC-{case}
if __name__ == "__main__":
    main()
"""
# if __name__ == "__main__": main()
def main():
    raise AssertionError("discovery must not execute source")
''')
            self.assertEqual(check_changed.citing({case}, root), [])
            self.assertEqual(check_changed.uncited({case}, root), [case])
            runner = scripts / "e2e_real.py"
            runner.write_text(f'# AC-{case}\nif (__name__ == \'__main__\'):\n    raise AssertionError("do not execute")\n')
            self.assertEqual(check_changed.citing({case}, root), ["scripts/e2e_real.py"])
            self.assertEqual(check_changed.uncited({case}, root), [])
            got = dict(check_changed.plan(["scripts/e2e_helper.py"], {case}, root=root))
            self.assertEqual(got["e2e_real"], "python3 scripts/e2e_real.py")
            self.assertNotIn("e2e_helper", got)

    def test_a_malformed_e2e_runner_is_not_silently_excluded(self):  # AC-104
        with tempfile.TemporaryDirectory(dir=short_tmp()) as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / "scripts/e2e_broken.py").write_text('if __name__ == "__main__":\n    main(\n')
            with self.assertRaises(SyntaxError):
                check_changed.citing({"104"}, root)
            with self.assertRaises(SyntaxError):
                check_changed.plan(["scripts/e2e_broken.py"], set(), root=root)

    def test_a_helper_of_a_page_check_runs_with_it(self):
        self.assertIn("webApp", steps(["web/static/x.js"], ["web/test/e2e_appscreen.mjs"])["page"])

    def test_a_new_case_is_found_and_its_tests_looked_for(self):
        new = "7" + "77"  # spelled so this file does not cite it
        diff = f"+### AC-{new} — A thing [SHOULD / P1]\n### AC-01 — Old\n-### AC-02 — Gone\n"
        self.assertEqual(check_changed.new_cases(diff), {new})
        self.assertEqual(check_changed.uncited({new}, REPO), [new])
        self.assertEqual(check_changed.uncited({"104"}, REPO), [])

    def test_every_plan_is_the_full_run_less(self):
        got = steps(["terminumd/src/hosts/tunnel.rs"])
        self.assertNotIn("make check", " ".join(got.values()))


class Land(unittest.TestCase):
    def test_a_landings_smoke_set_is_the_basics_on_one_platform(self):  # AC-104
        got = dict(check_changed.land_plan(["terminumd/src/hosts/tunnel.rs", "docs/USAGE.md"]))
        self.assertEqual(list(got), ["security", "ui", "debug-build", "cargo", "e2e_core", "e2e_attach", "smoke"])
        self.assertEqual(got["smoke"], "node web/test/e2e_live.mjs chromium smoke")
        self.assertNotIn("hosts::tunnel", got["cargo"], "the tree's own tests passed before it landed")
        self.assertEqual(dict(check_changed.land_plan(["docs/USAGE.md"])), {"security": check_changed.SCAN})

    def test_builds_leave_room_for_other_trees(self):  # AC-104
        for got in (steps(["terminumd/src/hosts/tunnel.rs"]), dict(check_changed.land_plan(["terminumd/src/x.rs"]))):
            for name in ("debug-build", "cargo"):
                self.assertIn("nice -n 10 cargo", got[name])

    def test_only_timed_steps_wait_for_a_quiet_machine(self):  # AC-146
        self.assertEqual([n for n in ("security", "cargo", "panel", "page", "critical", "e2e_hooks", "smoke") if check_changed.timed(n)],
                         ["panel", "page", "critical", "e2e_hooks"])


class Run(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root, self.events = Path(tmp.name), []
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)   # its cargo steps take a slot of its own repository

    def test_a_cargo_step_runs_holding_a_heavy_build_slot_and_a_light_one_does_not(self):  # AC-104
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        from worktree import Turn
        held = {}
        def fake(guard, command, **_):
            with Turn(self.root, "terminum-heavy.0.slot") as other:
                held[command[-1]] = not other.take()
            return 0
        with mock.patch.dict("os.environ", {"TM_HEAVY_SLOTS": "1"}), mock.patch.object(check_changed, "run_guard", fake), \
                mock.patch("sys.stdout"):
            check_changed.run([("cargo", "cargo test"), ("e2e_core", "python3 e2e"), ("debug-build", "cargo build")], root=self.root, gate=False)
        self.assertEqual(held, {"cargo test": True, "python3 e2e": False, "cargo build": True})

    def run_steps(self, steps, busy=None, **kw):
        def fake(guard, command, **_):
            self.events.append(guard + command)
            return 0

        def gate(_):
            self.events.append("gate")
            return busy
        with mock.patch.object(check_changed, "run_guard", fake), \
                mock.patch.object(check_changed, "for_task", gate), mock.patch("sys.stdout"):
            return check_changed.run(steps, root=self.root, **kw)

    def test_a_machine_still_busy_skips_the_timed_steps_and_says_so_last(self):  # AC-146
        self.assertEqual(self.run_steps([("cargo", "true"), ("page", "true"), ("e2e_hooks", "true")], busy="load 31 over 16"), [])
        self.assertEqual([e if e == "gate" else e[-1] for e in self.events], ["true", "gate"])
        self.assertEqual(check_changed.note(self.root, "skipped"),
                         "targeted: SKIPPED timed checks, the machine busy for 15 minutes: page e2e_hooks")

    def test_timed_steps_that_ran_clear_an_earlier_skip(self):  # AC-146
        self.run_steps([("page", "true")], busy="load 31 over 16")
        self.run_steps([("cargo", "true")])
        self.assertIsNotNone(check_changed.note(self.root, "skipped"))   # nothing timed ran: the skip stands
        self.run_steps([("e2e_core", "true")], gate=False)   # a landing's smoke set asks no gate
        self.assertIsNotNone(check_changed.note(self.root, "skipped"))
        self.run_steps([("page", "true")])
        self.assertIsNone(check_changed.note(self.root, "skipped"))

    def test_what_needs_no_quiet_machine_runs_before_the_wait(self):  # AC-146
        self.run_steps([("panel", "a"), ("page-unit", "b"), ("e2e_hooks", "c")])
        self.assertEqual([e if e == "gate" else e[-1] for e in self.events], ["b", "gate", "a", "c"])

    def test_the_first_timed_step_waits_once_for_a_quiet_machine(self):  # AC-146
        self.run_steps([("cargo", "true"), ("page", "true"), ("e2e_hooks", "true")])
        self.assertEqual([e if e == "gate" else e[-1] for e in self.events], ["true", "gate", "true", "true"])

    def test_a_landings_steps_never_wait_and_yield_during_a_release(self):  # AC-104, AC-146
        self.run_steps([("e2e_core", "true")], gate=False, nice=True)
        self.assertEqual(self.events[0][:3], ["nice", "-n", "10"])
        self.assertNotIn("gate", self.events)

    def run_codes(self, steps, codes, **kw):
        """Runs steps whose runs answer codes in turn: (failed, commands run, what was said)."""
        ran, answers = [], iter(codes)

        def fake(guard, command, stdout, **_):
            ran.append(command[-1])
            stdout.write(f"run {len(ran)}\n".encode())
            return next(answers)
        with mock.patch.object(check_changed, "run_guard", fake), mock.patch("sys.stdout") as out:
            failed = check_changed.run(steps, root=self.root, gate=False, **kw)
        return failed, ran, "".join(c.args[0] for c in out.write.call_args_list)

    def test_ac197_a_timed_step_red_once_passes_on_its_unchanged_retry_and_says_so_last(self):  # AC-197
        failed, ran, said = self.run_codes([("critical", "node live")], [1, 0])
        self.assertEqual((failed, ran), ([], ["node live", "node live"]))
        self.assertEqual((self.root / ".checks" / "changed.critical.first.log").read_text(), "run 1\n")
        self.assertIn("     run 1\n", said)   # the red run's lines are said: make land deletes the tree's logs
        self.assertEqual((self.root / ".checks" / "changed.critical.log").read_text(), "run 2\n")
        self.assertEqual(check_changed.note(self.root, "retried"),
                         "PASSED ONLY ON RETRY, the first run red: critical: name each with its open task\n"
                         "critical: the first run's last lines:\n     run 1")
        self.assertEqual(check_changed.late_notes(self.root), check_changed.note(self.root, "retried"))

    def test_ac197_red_again_on_its_retry_fails_and_says_no_retry_pass(self):  # AC-197
        failed, ran, _ = self.run_codes([("panel", "node panel")], [1, 1])
        self.assertEqual((failed, len(ran)), (["panel"], 2))
        self.assertIsNone(check_changed.note(self.root, "retried"))

    def test_ac197_a_step_that_measures_no_time_gets_no_retry(self):  # AC-197
        failed, ran, _ = self.run_codes([("cargo", "cargo test"), ("page-unit", "node --test")], [1, 1])
        self.assertEqual((failed, ran), (["cargo", "page-unit"], ["cargo test", "node --test"]))

    def test_ac197_a_run_red_already_or_a_leftover_gets_no_retry(self):  # AC-197, AC-102
        failed, ran, _ = self.run_codes([("cargo", "cargo test"), ("critical", "node live")], [1, 1])
        self.assertEqual((failed, ran), (["cargo", "critical"], ["cargo test", "node live"]))
        failed, ran, _ = self.run_codes([("e2e_core", "python3 core")], [check_changed.LEFT])
        self.assertEqual((failed, ran), (["e2e_core"], ["python3 core"]))

    def test_ac197_a_landings_smoke_step_retries_and_adds_to_the_targeted_runs_note(self):  # AC-197
        self.run_codes([("critical", "a")], [1, 0])
        failed, ran, _ = self.run_codes([("smoke", "node smoke")], [1, 0], landing=True)
        self.assertEqual((failed, len(ran)), ([], 2))
        note = check_changed.note(self.root, "retried")
        self.assertEqual(note.count("PASSED ONLY ON RETRY"), 2)
        self.assertIn("critical: the first run's last lines:\n     run 1", note)
        self.assertIn("smoke: the first run's last lines:\n     run 1", note)
        self.run_codes([("e2e_core", "b")], [0], landing=True)   # a landing with no retry keeps what was said
        self.assertIsNotNone(check_changed.note(self.root, "retried"))

    def test_ac197_a_new_targeted_run_says_only_its_own_retries(self):  # AC-197
        self.run_codes([("critical", "a")], [1, 0])
        self.run_codes([("critical", "a")], [0])
        self.assertIsNone(check_changed.note(self.root, "retried"))

class Passed(unittest.TestCase):
    def main_with(self, tmp, run, ids, argv=()):
        with mock.patch.object(check_changed, "REPO", Path(tmp)), \
                mock.patch.object(check_changed, "tree_id", side_effect=ids), \
                mock.patch.object(check_changed, "changed_files", return_value=(["docs/x.md"], "HEAD")), \
                mock.patch.object(check_changed, "touched_cases", return_value=set()), \
                mock.patch.object(check_changed, "run", run), mock.patch("sys.stdout") as out:
            (Path(tmp) / ".checks").mkdir(exist_ok=True)
            return check_changed.main(list(argv)), "".join(c.args[0] for c in out.write.call_args_list)

    def test_the_same_content_runs_once(self):  # AC-104
        with tempfile.TemporaryDirectory() as tmp:
            ran = []
            run = lambda steps, **_: ran.append(steps) or []   # noqa: E731
            same = ["3f9a21c40b7d5e11"] * 9
            self.assertEqual(self.main_with(tmp, run, same, ["--passed"])[0], 1)
            self.assertEqual(self.main_with(tmp, run, same)[0], 0)
            code, said = self.main_with(tmp, run, same)
            self.assertEqual((code, len(ran)), (0, 1))
            self.assertIn("targeted: passed already on 3f9a21c40b7d5e11", said)
            self.assertEqual(self.main_with(tmp, run, same, ["--passed"])[0], 0)

    def test_a_pass_that_skipped_its_timed_checks_says_so_again(self):  # AC-146
        with tempfile.TemporaryDirectory() as tmp:
            run = lambda steps, **_: []   # noqa: E731
            same = ["3f9a21c40b7d5e11"] * 9
            self.main_with(tmp, run, same)
            (Path(tmp) / ".checks" / "skipped").write_text("targeted: SKIPPED timed checks, the machine busy for 15 minutes: page\n")
            code, said = self.main_with(tmp, run, same)
            self.assertEqual(code, 0)
            self.assertRegex(said, r"targeted: SKIPPED timed checks, the machine busy for 15 minutes: page(\x1b\[0m)?\nnote: --force runs them")

    def test_a_red_run_records_no_pass(self):  # AC-104
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(self.main_with(tmp, lambda steps, **_: ["page"], ["aaaa", "aaaa"])[0], 1)
            self.assertEqual(list((Path(tmp) / ".checks").glob("changed-*.ok")), [])

    def test_content_changed_during_the_run_records_no_pass(self):  # AC-104
        with tempfile.TemporaryDirectory() as tmp:
            self.assertEqual(self.main_with(tmp, lambda steps, **_: [], ["aaaa", "bbbb"])[0], 0)
            self.assertEqual(list((Path(tmp) / ".checks").glob("changed-*.ok")), [])

    def test_the_id_is_the_content_not_the_commit(self):  # AC-104
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            subprocess.run(["git", "init", "-q", tmp], check=True)
            (root / ".gitignore").write_text(".checks/\n")
            (root / "a.txt").write_text("one\n")
            first = check_changed.tree_id(root)
            self.assertRegex(first, r"^[0-9a-f]{16}$")
            subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.com", "-C", tmp, "add", "-A"], check=True)
            subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.com", "-C", tmp, "commit", "-qm", "x"], check=True)
            self.assertEqual(check_changed.tree_id(root), first, "a commit keeps the content")
            (root / ".checks").mkdir()
            (root / ".checks" / "changed-x.ok").write_text("")
            self.assertEqual(check_changed.tree_id(root), first, "what is ignored is no content")
            (root / "a.txt").write_text("two\n")
            self.assertNotEqual(check_changed.tree_id(root), first, "an edit")
            (root / "a.txt").write_text("one\n")
            (root / "new.txt").write_text("")
            self.assertNotEqual(check_changed.tree_id(root), first, "a new file")


class SharedPass(unittest.TestCase):
    def repo(self, tmp):
        """A checkout and a second worktree of it, the same content in both."""
        main, other = Path(tmp) / "main", Path(tmp) / "other"
        run = lambda *a, cwd: subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@example.com", *a], cwd=cwd, check=True, capture_output=True)  # noqa: E731
        main.mkdir()
        run("init", "-q", cwd=main)
        (main / "a.txt").write_text("one\n")
        run("add", "a.txt", cwd=main)
        run("commit", "-qm", "one", cwd=main)
        run("worktree", "add", "-q", "--detach", str(other), "HEAD", cwd=main)
        return main, other

    def test_a_clean_pass_on_one_trees_content_is_found_by_another_tree_with_the_same_content(self):  # AC-198
        with tempfile.TemporaryDirectory(dir=short_tmp()) as tmp:
            main, other = self.repo(tmp)
            ident = check_changed.tree_id(main)
            self.assertEqual(check_changed.tree_id(other), ident, "the same content, the same id")
            self.assertFalse(check_changed.has_passed(ident, other))
            check_changed.keep_pass(ident, main)
            self.assertTrue(check_changed.has_passed(ident, other), "kept in git's common folder, found from the other tree")
            (other / "a.txt").write_text("two\n")
            self.assertFalse(check_changed.has_passed(check_changed.tree_id(other), other), "other content: run again")

    def test_a_pass_with_a_warning_stays_its_trees_own(self):  # AC-146, AC-197, AC-198
        for kind in ("skipped", "retried"):
            with tempfile.TemporaryDirectory(dir=short_tmp()) as tmp:
                main, other = self.repo(tmp)
                ident = check_changed.tree_id(main)
                (main / ".checks").mkdir()
                (main / ".checks" / kind).write_text("targeted: SKIPPED timed checks\n")
                check_changed.keep_pass(ident, main)
                self.assertTrue(check_changed.has_passed(ident, main), kind)
                self.assertFalse(check_changed.has_passed(ident, other), f"{kind}: its warning is said where it was kept, never passed on")


class OwnPass(unittest.TestCase):
    """A tree that merged the target into the commit it checks: terminumd's Verify after Prepare (AC-198)."""
    def setUp(self):
        tmp = tempfile.TemporaryDirectory(dir=short_tmp())
        self.addCleanup(tmp.cleanup)
        self.main, self.task = Path(tmp.name) / "main", Path(tmp.name) / "task"
        self.main.mkdir()
        git("init", "-q", cwd=self.main)
        self.write(self.main, ".gitignore", ".checks/\n")   # as the repository's: a run's notes are no content
        self.write(self.main, "a.txt", "1\n2\n3\n4\n5\n")
        self.write(self.main, "b.txt", "one\n")
        self.write(self.main, "scripts/land.py", "one\n")
        self.commit(self.main, "one")
        git("worktree", "add", "-q", "--detach", str(self.task), "HEAD", cwd=self.main)
        self.write(self.task, "a.txt", "task\n2\n3\n4\n5\n")
        self.sha = self.commit(self.task, "the task")

    def write(self, root, name, text):
        (root / name).parent.mkdir(parents=True, exist_ok=True)
        (root / name).write_text(text)

    def commit(self, root, message):
        git("add", "-A", cwd=root)
        git("commit", "-qm", message, cwd=root)
        return git("rev-parse", "HEAD", cwd=root)

    def merged(self, name, text="target\n", passed=True):
        """The task's commit, its pass kept (or not), merged with a target that changed `name`: the target's commit."""
        if passed:
            check_changed.keep_pass(check_changed.tree_id(self.task), self.task)
        self.write(self.main, name, text)
        base = self.commit(self.main, f"target: {name}")
        git("merge", "-q", "--no-edit", base, cwd=self.task)
        return base

    def test_a_clean_commits_id_is_its_tree(self):  # AC-198
        self.assertEqual(check_changed.tree_id(self.task), git("rev-parse", "HEAD^{tree}", cwd=self.task)[:16])

    def test_an_edit_hidden_by_assume_unchanged_is_content(self):  # AC-104
        clean = check_changed.tree_id(self.task)
        git("update-index", "--assume-unchanged", "b.txt", cwd=self.task)
        self.write(self.task, "b.txt", "edited\n")
        self.assertNotEqual(check_changed.tree_id(self.task), clean)

    def test_the_commits_pass_counts_when_the_target_changed_none_of_its_files(self):  # AC-198
        base = self.merged("b.txt")
        self.assertEqual(check_changed.own_pass(base, self.task), f"targeted: own pass on {self.sha[:9]} reused: {base} changed none of its files")

    def test_no_pass_or_a_pass_with_a_warning_elsewhere_counts_for_nothing(self):  # AC-146, AC-197, AC-198
        self.assertIsNone(check_changed.own_pass(self.merged("b.txt", passed=False), self.task), "no pass")
        other = self.task.parent / "other"
        git("worktree", "add", "-q", "--detach", str(other), self.sha, cwd=self.main)
        (other / ".checks").mkdir()
        (other / ".checks" / "retried").write_text("targeted: PASSED ON RETRY page\n")
        check_changed.keep_pass(check_changed.tree_id(other), other)
        self.assertIsNone(check_changed.own_pass("HEAD^2", self.task), "passed only on a retry: that tree's own")

    def test_the_target_changing_one_of_its_files_or_a_check_runs_them_again(self):  # AC-198
        self.assertIsNone(check_changed.own_pass(self.merged("a.txt", "1\n2\n3\n4\ntarget\n"), self.task), "the same file")
        git("reset", "-q", "--hard", self.sha, cwd=self.task)
        self.assertIsNone(check_changed.own_pass(self.merged("scripts/land.py"), self.task), "a check")

    def test_an_edit_after_the_merge_or_no_merge_reuses_nothing(self):  # AC-198
        base = self.merged("b.txt")
        self.write(self.task, "b.txt", "uncommitted\n")
        self.assertIsNone(check_changed.own_pass(base, self.task), "uncommitted: not the merge alone")
        git("checkout", "-q", "--", "b.txt", cwd=self.task)
        git("reset", "-q", "--hard", self.sha, cwd=self.task)
        self.assertIsNone(check_changed.own_pass(base, self.task), "no merge of the target")

    def test_the_checks_say_the_reuse_and_run_nothing(self):  # AC-198
        base = self.merged("b.txt")
        def main(argv, run):
            with mock.patch.object(check_changed, "REPO", self.task), mock.patch.object(check_changed, "run", run), \
                    mock.patch.object(check_changed, "tree_id", return_value="merged0content00"), \
                    mock.patch.object(check_changed, "changed_files", return_value=(["docs/x.md"], base)), \
                    mock.patch.object(check_changed, "touched_cases", return_value=set()), redirect_stdout(io.StringIO()) as said:
                return check_changed.main(["--base", base, *argv]), said.getvalue()
        code, said = main([], mock.Mock(side_effect=AssertionError("ran")))
        self.assertEqual(code, 0)
        self.assertIn(f"targeted: own pass on {self.sha[:9]} reused", said)
        ran = mock.Mock(return_value=[])
        self.assertEqual(main(["--force"], ran)[0], 0)
        ran.assert_called_once()

if __name__ == "__main__":
    unittest.main()
