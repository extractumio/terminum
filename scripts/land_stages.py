#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The seven stages of a landing (docs/specs/terminum-integration.md AC-198), each a command of its own, each
idempotent: run again it does nothing when its work is done, and goes on otherwise. terminumd's integration queue
runs them by a project's settings; without it `land.py <slug>` runs them in order (scripts/land.py).

  land.py prepare <slug>   the target merged into the tree; a conflict stays there to resolve (exit 2)
  land.py verify  <slug>   the targeted checks on the tree's content (kept per content)
  land.py queue   <slug>   with integration on: queued by terminum (TASK, RESULT); off: the landing turn waits
  land.py check  [<slug>]  the smoke set on the merged result, its green kept for that commit on that base
  land.py publish <slug>   pushed, never forced, only a commit checked green on the target as it is now
  land.py after            the primary checkout on the fresh target and this build installed
  land.py cleanup          landed trees removed by worktree.py sweep, never one with work or a process in it

With no <slug>, check runs in the folder it is started in (terminumd's candidate tree). Exit: 0 done; 1 failed
or refused; 2 a conflict left in the tree."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import land  # noqa: E402
from land import BASE, Stop, git  # noqa: E402

STAGES = ("prepare", "verify", "queue", "check", "publish", "after", "cleanup")


def terminum():
    """The terminum command: on PATH, else where its install puts it."""
    return shutil.which("terminum") or str(Path.home() / ".terminum" / "bin" / "terminum")


def green_mark(tree, commit, base):
    """Where a green check of `commit` on `base` is kept, in git's common folder beside the landing turn, for
    every tree of the repository: Publish pushes nothing else."""
    return land.common_dir(tree) / "terminum-land-green" / f"{commit[:16]}-{base[:16]}"


def head(tree):
    return git("rev-parse", "HEAD", cwd=tree).stdout.strip()


def base_of(tree):
    return git("rev-parse", BASE, cwd=tree).stdout.strip()


def prepare(tree, say=print):
    """Stage 1: committed work, the fresh target merged in. Done when the target is in HEAD."""
    if git("status", "--porcelain", cwd=tree).stdout.strip():
        raise Stop(1, f"{tree} has uncommitted changes: commit them first")
    git("fetch", "--quiet", "origin", cwd=tree)
    if land.has_base(tree):
        say(f"prepare: {BASE} is in {Path(tree).name} already")
        return
    land.merge_base_in(tree)
    say(f"prepare: merged {BASE} ({base_of(tree)[:9]})")


def verify(tree, check=land.checks_of, passed=lambda tree: land.checks_of(tree, "--passed"), say=print):
    """Stage 2: the targeted checks on the tree's content; a pass recorded for this content is not run again."""
    if passed(tree):
        say("verify: passed already on this content")
        return
    if not check(tree):
        raise Stop(1, f"verify: targeted checks failed in {tree}")
    say("verify: passed")


def integration_on(tree, run=None):
    """Whether terminum queues this project's landings: its integration_status, as the tree's session calls it;
    None when terminum is not reached (the stages then run here, in order)."""
    run = run or subprocess.run
    try:
        done = run([terminum(), "tool", "integration_status", "{}"], cwd=tree, capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if done.returncode:
        return None
    try:
        return bool(json.loads(done.stdout)["on"])
    except (ValueError, KeyError, TypeError):
        return None


def queue(tree, task=None, result=None, on=integration_on, run=None, say=print):
    """Stage 3: with integration on, the tree's HEAD queued by terminum for task `task` with its result file, and
    waited for (exit 0 landed, 1 red, 2 conflict, 3 taken out); off, nothing to queue: check and publish take the
    landing turn. Done when the task is queued or landed: terminum says so."""
    if not on(tree):
        say("queue: integration is off: the landing turn orders check and publish")
        return 0
    if not task or not result:
        raise Stop(1, "queue: integration is on: give TASK=<ID> and RESULT=<file with the result finish_task "
                      "would take>, or queue from the session with integrate {id, result}")
    done = (run or subprocess.run)([terminum(), "integration", "queue", task, "--result-file", str(result), "--commit", head(tree), "--wait"], cwd=tree)
    return done.returncode


def check(tree, smoke=lambda tree: land.checks_of(tree, "--land", minutes=land.SMOKE_MOST_MINUTES), base=None, say=print):
    """Stage 4: the smoke set on the tree as it is (merged with the target as last fetched: in terminumd's candidate
    tree, the one its batch was built on); its green kept for that commit on that base. Done when that green is
    kept."""
    given = base
    commit, base = head(tree), base or base_of(tree)
    mark = green_mark(tree, commit, base)
    if mark.exists():
        say(f"check: {commit[:9]} is green on {base[:9]} already")
        return
    # terminumd's candidate is built on its batch's base, which it names; a tree is to be prepared first
    if not given and not land.has_base(tree):
        raise Stop(1, f"check: {BASE} is not in {Path(tree).name}: prepare first")
    if not smoke(tree):
        raise Stop(1, f"check: smoke checks failed on {commit[:9]}")
    mark.parent.mkdir(parents=True, exist_ok=True)
    mark.write_text(f"{commit} {base}\n")
    say(f"check: {commit[:9]} green on {base[:9]}")


def publish(tree, say=print):
    """Stage 5: HEAD pushed to the target, never forced, only when it was checked green on the target as it is
    now. Done when the target holds it."""
    git("fetch", "--quiet", "origin", cwd=tree)
    commit, base = head(tree), base_of(tree)
    if git("merge-base", "--is-ancestor", commit, BASE, cwd=tree, check=False).returncode == 0:
        say(f"publish: {commit[:9]} is in {BASE} already")
        return commit
    if git("status", "--porcelain", cwd=tree).stdout.strip():
        raise Stop(1, f"publish: {tree} has uncommitted changes: nothing pushed")
    if not green_mark(tree, commit, base).exists():
        moved = not land.has_base(tree)
        raise Stop(1, f"publish: {commit[:9]} " + (f"is not on {BASE} as it is now ({base[:9]}): prepare and check again"
                                                   if moved else f"has no green check on {base[:9]}: check first"))
    pushed = git("push", "--quiet", "origin", f"{commit}:main", cwd=tree, check=False)
    if pushed.returncode:
        raise Stop(1, f"publish: push refused: {pushed.stderr.strip()}")
    git("fetch", "--quiet", "origin", cwd=tree)
    say(f"publish: {commit[:9]} on {BASE}")
    return commit


def after(primary, run=land.make, say=print):
    """Stage 6: the primary checkout on the fresh target, then this build installed, in the install turn and a
    heavy-build slot. Done when `make installed` says this tree's build runs."""
    if land.releasing(primary):
        raise Stop(1, "after: a release is running: the primary checkout stays where it is")
    stuck = land.update_primary(primary)
    if stuck:
        raise Stop(1, f"after: the primary checkout stays where it is: {stuck}")
    if run(primary, "installed").returncode == 0:
        say("after: this build is installed already")
        return
    with land.TreeTurn(primary, "terminum-install.turn") as turn:
        turn.take(wait=True)
        with land.heavy_slots.slot(primary, "install", say=say):
            if not land.install_steps(primary, run, say):
                raise Stop(1, "after: the install failed")


def cleanup(primary, run=None):
    """Stage 7: trees that landed, idle, with nothing of their own left, removed as worktree.py sweep removes
    them; never one with work or a process in it. Done when none is left to take."""
    return (run or subprocess.run)([sys.executable, str(Path(primary) / "scripts" / "worktree.py"), "sweep"], cwd=primary).returncode


def run_stage(stage, slug, primary, env=os.environ):
    """One stage as `land.py <stage> [<slug>]` names it: its exit status."""
    tree = (primary / ".worktrees" / slug) if slug else Path.cwd()
    if slug and not tree.is_dir():
        raise Stop(1, f"no worktree {tree}")
    if stage == "prepare":
        prepare(tree)
    elif stage == "verify":
        verify(tree)
    elif stage == "queue":
        return queue(tree, env.get("TASK"), env.get("RESULT"))
    elif stage == "check":
        check(tree, base=None if slug else env.get("TM_INTEGRATION_BASE"))
    elif stage == "publish":
        land.unchanged(tree, land.tree_state(tree))   # the tree's state now: nothing is adopted meanwhile
        with land.Turn(tree):
            publish(tree)
    elif stage == "after":
        after(primary)
    else:
        return cleanup(primary)
    return 0
