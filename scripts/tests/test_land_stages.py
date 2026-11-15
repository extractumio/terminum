# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The seven stages of a landing (spec AC-198), each alone and run again, on real repositories in a temporary
folder; no remote is contacted, no terminum is called."""
from pathlib import Path
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import test_land  # noqa: E402
from test_land import git  # noqa: E402
import land_stages as stages  # noqa: E402

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
        stages.prepare(tree, **QUIET)
        merged = git("rev-parse", "HEAD", cwd=tree)
        self.assertTrue(stages.land.has_base(tree))
        stages.prepare(tree, **QUIET)
        self.assertEqual(git("rev-parse", "HEAD", cwd=tree), merged, "run again: nothing to do")
        (tree / "loose.txt").write_text("z\n")
        with self.assertRaises(stages.Stop) as stop:
            stages.prepare(tree, **QUIET)
        self.assertIn("uncommitted", str(stop.exception))

    def test_prepare_leaves_a_conflict_in_the_tree_exit_2(self):  # AC-198
        tree = self.tree("one")
        self.commit(tree, "a.txt", "ours\n")
        self.push_from_outside("a.txt", "theirs\n")
        with self.assertRaises(stages.Stop) as stop:
            stages.prepare(tree, **QUIET)
        self.assertEqual(stop.exception.code, 2)
        self.assertIn("a.txt", str(stop.exception))

    def test_verify_runs_once_per_content(self):  # AC-198
        tree, ran = self.ready(), []
        stages.verify(tree, check=lambda t: ran.append(t) or True, passed=lambda t: bool(ran), **QUIET)
        stages.verify(tree, check=lambda t: ran.append(t) or True, passed=lambda t: bool(ran), **QUIET)
        self.assertEqual(len(ran), 1, "a pass kept for this content is not run again")
        with self.assertRaises(stages.Stop):
            stages.verify(tree, check=lambda t: False, passed=lambda t: False, **QUIET)

    def test_check_keeps_its_green_for_that_commit_on_that_base_and_needs_prepare(self):  # AC-198
        tree, smokes = self.ready(), []
        smoke = lambda t: smokes.append(t) or True
        git("fetch", "-q", "origin", cwd=tree)   # check judges the target as last fetched: a candidate's is its batch's
        with self.assertRaises(stages.Stop) as stop:
            stages.check(tree, smoke=smoke, **QUIET)
        self.assertIn("prepare first", str(stop.exception))
        stages.prepare(tree, **QUIET)
        stages.check(tree, smoke=smoke, **QUIET)
        stages.check(tree, smoke=smoke, **QUIET)
        self.assertEqual(len(smokes), 1, "green kept: not run again")
        self.assertTrue(stages.green_mark(tree, stages.head(tree), stages.base_of(tree)).exists())
        # terminumd's candidate: its batch's base named, whatever origin/main is meanwhile
        self.push_from_outside("later.txt", "w\n")
        git("fetch", "-q", "origin", cwd=tree)
        stages.check(tree, smoke=smoke, base="b" * 40, **QUIET)
        self.assertEqual(len(smokes), 2)
        self.assertTrue(stages.green_mark(tree, stages.head(tree), "b" * 40).exists())

    def test_publish_pushes_only_a_commit_checked_green_on_the_target_as_it_is(self):  # AC-198
        tree = self.ready()
        stages.prepare(tree, **QUIET)
        with self.assertRaises(stages.Stop) as stop:
            stages.publish(tree, **QUIET)
        self.assertIn("has no green check", str(stop.exception))
        stages.check(tree, smoke=lambda t: True, **QUIET)
        self.push_from_outside("later.txt", "w\n")   # the target moved after the check
        with self.assertRaises(stages.Stop) as stop:
            stages.publish(tree, **QUIET)
        self.assertIn("prepare and check again", str(stop.exception))
        before = self.on_origin()
        self.assertEqual(self.on_origin(), before, "nothing pushed")
        stages.prepare(tree, **QUIET)
        stages.check(tree, smoke=lambda t: True, **QUIET)
        made = stages.publish(tree, **QUIET)
        self.assertEqual(self.on_origin(), made)
        self.assertEqual(stages.publish(tree, **QUIET), made, "run again: in the target already")

    def test_queue_with_integration_off_leaves_the_turn_to_check_and_publish(self):  # AC-198
        tree = self.ready()
        said = []
        self.assertEqual(stages.queue(tree, on=lambda t: False, say=said.append), 0)
        self.assertIn("integration is off", said[0])

    def test_queue_with_integration_on_needs_its_task_and_result_and_queues_head_and_waits(self):  # AC-198
        tree, calls = self.ready(), []
        with self.assertRaises(stages.Stop) as stop:
            stages.queue(tree, on=lambda t: True, **QUIET)
        self.assertIn("TASK=<ID> and RESULT=", str(stop.exception))
        run = lambda argv, cwd: calls.append(argv) or subprocess.CompletedProcess(argv, 1)
        self.assertEqual(stages.queue(tree, "API-12", "/r.md", on=lambda t: True, run=run, **QUIET), 1, "the wait's exit: red")
        self.assertEqual(calls[0][1:], ["integration", "queue", "API-12", "--result-file", "/r.md", "--commit", stages.head(tree), "--wait"])

    def test_integration_on_reads_terminums_answer_and_none_when_it_is_not_reached(self):  # AC-198
        answer = lambda out, code=0: (lambda argv, **_: subprocess.CompletedProcess(argv, code, stdout=out))
        tree = self.tree("one")
        self.assertTrue(stages.integration_on(tree, run=answer('{"on": true, "queue": []}')))
        self.assertFalse(stages.integration_on(tree, run=answer('{"on": false}')))
        self.assertIsNone(stages.integration_on(tree, run=answer("terminum is not running", 1)))
        self.assertIsNone(stages.integration_on(tree, run=answer("not json")))

    def test_cleanup_is_worktree_py_sweep(self):  # AC-198
        calls = []
        stages.cleanup(self.primary, run=lambda argv, cwd: calls.append((argv[1:], cwd)) or subprocess.CompletedProcess(argv, 0))
        self.assertEqual(calls, [([str(self.primary / "scripts" / "worktree.py"), "sweep"], self.primary)])


if __name__ == "__main__":
    unittest.main()
