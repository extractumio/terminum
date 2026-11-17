# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The seven stages of a landing (spec AC-198), each alone and run again, on real repositories in a temporary
folder; no remote is contacted, no terminum is called."""
import os
from pathlib import Path
import subprocess
import sys
import threading
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import test_land  # noqa: E402
from test_land import git  # noqa: E402
import land_stages as stages  # noqa: E402
from worktree import mark_path  # noqa: E402
L = stages.land

QUIET = dict(say=lambda _: None)


class Stages(test_land.Repos, unittest.TestCase):
    def ready(self, slug="one"):
        """A tree with work of its own, origin/main moved meanwhile by a push from outside."""
        tree = self.tree(slug)
        self.commit(tree, "new.txt", "x\n")
        self.push_from_outside("other.txt", "y\n")
        return tree

    def test_prepare_merges_the_target_once_and_refuses_uncommitted_work(self):  # AC-198
        tree = self.ready()
        self.assertTrue(L.prepare(tree, **QUIET))
        merged = git("rev-parse", "HEAD", cwd=tree)
        self.assertFalse(L.prepare(tree, **QUIET), "run again: nothing to do")
        self.assertEqual(git("rev-parse", "HEAD", cwd=tree), merged)
        (tree / "loose.txt").write_text("z\n")
        with self.assertRaises(stages.Stop) as stop:
            L.prepare(tree, **QUIET)
        self.assertIn("uncommitted", str(stop.exception))

    def test_prepare_waits_for_a_ref_another_fetch_holds_and_fails_red_when_it_stays_held(self):  # AC-198
        tree = self.ready()
        lock = Path(git("rev-parse", "--path-format=absolute", "--git-common-dir", cwd=tree)) / "refs/remotes/origin/main.lock"
        lock.write_text("")   # another tree's fetch holds it for a second
        threading.Timer(1, lock.unlink).start()
        self.assertTrue(L.prepare(tree, **QUIET), "waited for it, then merged")
        self.push_from_outside("more.txt", "z\n")
        lock.write_text("")   # held for good: a stale lock of a killed fetch
        before = git("rev-parse", "HEAD", cwd=tree)
        with self.assertRaises(stages.Stop) as stop:
            L.prepare(tree, **QUIET)
        lock.unlink()
        self.assertEqual(stop.exception.code, 1, "red, never a conflict")
        self.assertIn("git fetch origin:", str(stop.exception))
        self.assertIn("lock", str(stop.exception))
        self.assertEqual(git("rev-parse", "HEAD", cwd=tree), before, "nothing merged")

    def test_prepare_leaves_a_conflict_in_the_tree_exit_2(self):  # AC-198
        tree = self.tree("one")
        self.commit(tree, "a.txt", "ours\n")
        self.push_from_outside("a.txt", "theirs\n")
        with self.assertRaises(stages.Stop) as stop:
            L.prepare(tree, **QUIET)
        self.assertEqual(stop.exception.code, 2)
        self.assertIn("a.txt", str(stop.exception))

    def test_verify_runs_the_targeted_checks_and_fails_red(self):  # AC-198
        tree, ran = self.ready(), []
        stages.verify(tree, check=lambda t: ran.append(t) or True, **QUIET)
        self.assertEqual(ran, [tree])
        with self.assertRaises(stages.Stop):
            stages.verify(tree, check=lambda t: False, **QUIET)

    def test_check_keeps_its_green_for_that_commit(self):  # AC-198
        tree, smokes = self.ready(), []
        smoke = lambda t: smokes.append(t) or True
        L.smoke_green(tree, smoke, **QUIET)
        L.smoke_green(tree, smoke, **QUIET)
        self.assertEqual(len(smokes), 1, "green kept: not run again")
        self.assertTrue(L.green_mark(tree, L.head(tree)).exists())
        self.commit(tree, "more.txt", "m\n")
        with self.assertRaises(stages.Stop) as stop:
            L.smoke_green(tree, lambda t: False, **QUIET)
        self.assertIn("smoke checks failed", str(stop.exception))

    def test_publish_pushes_only_a_commit_checked_green_holding_the_target_as_it_is(self):  # AC-198
        tree = self.ready()
        before = self.on_origin()
        with self.assertRaises(stages.Stop) as stop:
            L.publish(tree, **QUIET)
        self.assertIn("prepare and check again", str(stop.exception), "the target is not in it")
        L.prepare(tree, **QUIET)
        with self.assertRaises(stages.Stop) as stop:
            L.publish(tree, **QUIET)
        self.assertIn("has no green check", str(stop.exception))
        L.smoke_green(tree, lambda t: True, **QUIET)
        self.push_from_outside("later.txt", "w\n")   # the target moved after the check
        with self.assertRaises(L.Moved):
            L.publish(tree, **QUIET)
        self.assertNotEqual(self.on_origin(), git("rev-parse", "HEAD", cwd=tree), "nothing pushed")
        L.prepare(tree, **QUIET)
        L.smoke_green(tree, lambda t: True, **QUIET)
        made = L.publish(tree, **QUIET)
        self.assertEqual(self.on_origin(), made)
        self.assertNotEqual(made, before)
        self.assertEqual(L.publish(tree, **QUIET), made, "run again: in the target already")

    def test_queue_with_integration_off_leaves_the_turn_to_check_and_publish(self):  # AC-198
        tree = self.ready()
        said = []
        self.assertEqual(stages.queue(tree, on=lambda *_: False, say=said.append), 0)
        self.assertIn("integration is off", said[0])

    def test_queue_with_integration_on_needs_its_task_and_result_and_queues_head_and_waits(self):  # AC-198
        tree, calls = self.ready(), []
        with self.assertRaises(stages.Stop) as stop:
            stages.queue(tree, on=lambda *_: True, **QUIET)
        self.assertIn("TASK=<ID> and RESULT=", str(stop.exception))
        run = lambda argv, cwd: calls.append(argv) or subprocess.CompletedProcess(argv, 1)
        self.assertEqual(stages.queue(tree, "API-12", "/r.md", on=lambda *_: True, run=run, **QUIET), 1, "the wait's exit: red")
        self.assertEqual(calls[0][1:], ["integration", "queue", "API-12", "--result-file", "/r.md", "--commit", L.head(tree), "--wait"])

    def test_integration_on_reads_terminums_answer_none_when_it_does_not_run_else_refuses(self):  # AC-198
        asked = []
        answer = lambda out, code=0, err="": (lambda argv, **_: asked.append(argv) or subprocess.CompletedProcess(argv, code, stdout=out, stderr=err))
        tree = self.tree("one")
        self.assertTrue(stages.integration_on(tree, "API-12", run=answer('{"on": true, "queue": []}')))
        self.assertEqual(asked[-1][-1], '{"id": "API-12"}', "the task's own project, not the caller's")
        self.assertFalse(stages.integration_on(tree, run=answer('{"on": false}')))
        self.assertIsNone(stages.integration_on(tree, run=answer("", 1, "terminum is not running; go on without it.")))
        for bad in [answer("not json"), answer("", 1, "terminum tool runs in a session of terminum, or with a token in TERMINUM_TOKEN")]:
            with self.assertRaises(stages.Stop) as stop:
                stages.integration_on(tree, run=bad)
            self.assertIn("nothing pushed", str(stop.exception), "never pushed from the tree when it may be on")

    def test_publish_and_after_by_hand_are_refused_while_integration_is_on_but_not_from_terminumd(self):  # AC-198
        tree = self.ready()
        with self.assertRaises(stages.Stop) as stop:
            stages.not_while_queued(tree, "publish", env={}, on=lambda *_: True)
        self.assertIn("integration is on", str(stop.exception))
        stages.not_while_queued(tree, "after", env={"TM_INTEGRATION_BATCH": "7"}, on=lambda *_: True)
        stages.not_while_queued(tree, "publish", env={}, on=lambda *_: False)

    def test_check_refuses_a_tree_being_edited_and_marks_nothing_it_did_not_check_whole(self):  # AC-198
        tree = self.ready()
        (tree / "loose.txt").write_text("z\n")
        with self.assertRaises(stages.Stop) as stop:
            L.smoke_green(tree, lambda t: True, **QUIET)
        self.assertIn("uncommitted", str(stop.exception))
        (tree / "loose.txt").unlink()
        commit = L.head(tree)
        with self.assertRaises(stages.Stop) as stop:
            L.smoke_green(tree, lambda t: bool(self.commit(tree, "during.txt", "d\n")), **QUIET)
        self.assertIn("changed while landing, during its check", str(stop.exception))
        self.assertFalse(L.green_mark(tree, commit).exists())

    def test_queue_refuses_uncommitted_work_and_reads_its_result_from_where_it_was_run(self):  # AC-198
        tree, calls = self.ready(), []
        run = lambda argv, cwd: calls.append(argv) or subprocess.CompletedProcess(argv, 0)
        self.assertEqual(stages.queue(tree, "API-12", "result.md", on=lambda *_: True, run=run, **QUIET), 0)
        self.assertEqual(calls[0][5], str(Path("result.md").resolve()))
        (tree / "loose.txt").write_text("z\n")
        with self.assertRaises(stages.Stop) as stop:
            stages.queue(tree, "API-12", "result.md", on=lambda *_: True, run=run, **QUIET)
        self.assertIn("uncommitted", str(stop.exception))

    def test_cleanup_is_worktree_py_sweep(self):  # AC-198
        calls = []
        stages.cleanup(self.primary, run=lambda argv, cwd: calls.append((argv[1:], cwd)) or subprocess.CompletedProcess(argv, 0))
        self.assertEqual(calls, [([str(self.primary / "scripts" / "worktree.py"), "sweep"], self.primary)])



class After(test_land.Repos, unittest.TestCase):
    def test_after_moves_the_checkout_and_installs_only_where_the_maintainer_asked(self):  # AC-198
        made, said = [], []
        run = lambda primary, target: made.append(target) or subprocess.CompletedProcess([], 1 if target == "installed" and len(made) == 1 else 0, stdout="ok\n")
        stages.after(self.primary, run=run, env={"TM_LAND_INSTALL": ""}, say=said.append)
        self.assertEqual(made, [], "no TM_LAND_INSTALL: no install")
        self.assertIn("no install here", said[-1])
        stages.after(self.primary, run=run, env={"TM_LAND_INSTALL": "1"}, say=said.append)
        self.assertEqual(made, ["installed", "build", "install", "installed"], "installed, by the landings' own install")
        self.assertFalse(L.install_mark(self.primary).exists())

    def later(self, **kw):
        made = []
        with self.assertRaises(stages.Stop) as stop:
            stages.after(self.primary, run=lambda p, target: made.append(target), env={"TM_LAND_INSTALL": "1"}, say=lambda _: None, **kw)
        self.assertEqual((stop.exception.code, made), (stages.LATER, []), "nothing built, nothing installed")
        return str(stop.exception)

    def test_after_cannot_run_now_under_a_release_or_a_held_turn_ends_75_moving_nothing(self):  # AC-191, AC-198
        self.push_from_outside("later.txt", "w\n")
        L.git("fetch", "-q", "origin", cwd=self.primary)
        before = L.head(self.primary)
        mark = mark_path(self.primary)
        mark.write_text(f"{os.getpid()}\n")   # a live release
        self.assertEqual(self.later(), "after: a release is running: waits for its end")
        mark.unlink()
        for name, words in ((stages.INSTALL_TURN, "after: an install runs in the primary checkout: waits for its end"),
                            (L.LAND_TURN, "after: a landing holds its turn: waits for its end")):
            with stages.Turn(self.primary, name) as held:
                self.assertTrue(held.take())
                self.assertEqual(self.later(), words)
        self.assertEqual(L.head(self.primary), before, "the primary checkout stays where it is")
        stages.after(self.primary, run=lambda *_: subprocess.CompletedProcess([], 0), env={"TM_LAND_INSTALL": "1"}, say=lambda _: None)
        self.assertNotEqual(L.head(self.primary), before, "free again: it follows")

    def test_after_holds_the_install_turn_while_it_installs(self):  # AC-191, AC-104
        seen = []
        def run(primary, target):
            with stages.Turn(primary, stages.INSTALL_TURN) as other:
                seen.append(other.take())
            return subprocess.CompletedProcess([], 1 if target == "installed" and len(seen) == 1 else 0, stdout="ok\n")
        stages.after(self.primary, run=run, env={"TM_LAND_INSTALL": "1"}, say=lambda _: None)
        self.assertEqual(seen, [False] * 4, "a release finds it held from the first look to the install's end")

    def test_after_reads_whether_to_install_from_make_when_run_without_it(self):  # AC-191
        (self.primary / "Makefile").write_text("land-install-wanted:\n\t@echo 1\n")
        self.assertTrue(L.install_wanted(self.primary, env={}))
        self.assertFalse(L.install_wanted(self.primary, env={"TM_LAND_INSTALL": ""}), "the value make gave the line wins")
        (self.primary / "Makefile").write_text("land-install-wanted:\n\t@echo\n")
        self.assertFalse(L.install_wanted(self.primary, env={}))
        (self.primary / "Makefile").unlink()
        with self.assertRaisesRegex(stages.Stop, "after: whether to install is not known: make land-install-wanted: "):
            L.install_wanted(self.primary, env={})   # never a silent "no install"

    def test_an_install_that_waited_for_a_starting_release_installs_nothing(self):  # AC-104, AC-191
        L.install_mark(self.primary).write_text(L.head(self.primary))
        made, said = [], []
        def released_meanwhile(self_, wait=False):   # the release held the turn and wrote its mark in it
            mark_path(self.primary).write_text(f"{os.getpid()}\n")
            return True
        with mock.patch.object(stages.Turn, "take", released_meanwhile), mock.patch.object(L, "releasing", side_effect=[False, True]):
            self.assertEqual(L.install_pending(self.primary, run=lambda p, t: made.append(t), say=said.append), "a release is running")
        self.assertEqual(made, [])
        self.assertTrue(L.install_mark(self.primary).exists(), "kept for a landing after the release")


if __name__ == "__main__":
    unittest.main()
