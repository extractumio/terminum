#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The seven stages of a landing (docs/specs/terminum-integration.md AC-198), each a command of its own, each
idempotent: run again it does nothing when its work is done, and goes on otherwise. terminumd's integration queue
runs them by a project's settings; without it `land.py <slug>` runs them in order: land() of scripts/land.py, which
holds Prepare, Check and Publish in the landing turn.

  land.py prepare [<slug>]   the target merged into the tree; a conflict stays there to resolve (exit 2)
  land.py verify  [<slug>]   the targeted checks on the tree's content (kept per content)
  land.py queue   <slug>     with integration on: queued by terminum (TASK, RESULT); off: nothing to queue
  land.py check   [<slug>]   the smoke set on HEAD, its green kept for that commit
  land.py publish <slug>     pushed, never forced, only a commit checked green that holds the target as it is now
  land.py after              the primary checkout on the fresh target; with TM_LAND_INSTALL=1, this build installed
  land.py cleanup            landed trees removed by worktree.py sweep, never one with work or a process in it

With no <slug> a stage runs in the folder it is started in (terminumd's own trees). Exit: 0 done; 1 failed or
refused; 2 a conflict left in the tree."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import land  # noqa: E402
from land import Stop, git  # noqa: E402

STAGES = ("prepare", "verify", "queue", "check", "publish", "after", "cleanup")


def terminum():
    """The terminum command: on PATH, else where its install puts it."""
    return shutil.which("terminum") or str(Path.home() / ".terminum" / "bin" / "terminum")


def verify(tree, check=land.checks_of, say=print):
    """Stage 2: the targeted checks on the tree's content; check_changed.py itself skips a pass kept for it."""
    if not check(tree):
        raise Stop(1, f"verify: targeted checks failed in {tree}")
    say("verify: passed")


def integration_on(tree, run=None):
    """Whether terminum queues this project's landings: its integration_status, as the tree's session calls it;
    None when terminum is not reached (the stages then run here, in order)."""
    try:
        done = (run or subprocess.run)([terminum(), "tool", "integration_status", "{}"], cwd=tree, capture_output=True, text=True, timeout=30)
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
    landing turn."""
    if not on(tree):
        say("queue: integration is off: the landing turn orders check and publish")
        return 0
    if not task or not result:
        raise Stop(1, "queue: integration is on: give TASK=<ID> and RESULT=<file with the result finish_task "
                      "would take>, or queue from the session with integrate {id, result}")
    argv = [terminum(), "integration", "queue", task, "--result-file", str(result), "--commit", land.head(tree), "--wait"]
    return (run or subprocess.run)(argv, cwd=tree).returncode


def after(primary, run=land.make, env=os.environ, say=print):
    """Stage 6: the primary checkout on the fresh target; on the maintainer's machine (TM_LAND_INSTALL=1) this build
    installed, by the landings' own install (its turn, its heavy-build slot, its guard that the checkout is on what
    landed). Done when the checkout follows and `make installed` says this build runs."""
    if land.releasing(primary):
        raise Stop(1, "after: a release is running: the primary checkout stays where it is")
    stuck = land.update_primary(primary)
    if stuck:
        raise Stop(1, f"after: the primary checkout stays where it is: {stuck}")
    if env.get("TM_LAND_INSTALL") != "1":
        say("after: the primary checkout follows; no install here (TM_LAND_INSTALL is not 1)")
        return
    if run(primary, "installed").returncode == 0:
        say("after: this build is installed already")
        return
    land.install_mark(primary).write_text(land.head(primary))
    land.install_pending(primary, run=run, say=say)
    if land.install_mark(primary).exists():
        raise Stop(1, "after: the install did not run: see above")


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
        land.prepare(tree)
    elif stage == "verify":
        verify(tree)
    elif stage == "queue":
        return queue(tree, env.get("TASK"), env.get("RESULT"))
    elif stage == "check":
        land.smoke_green(tree)
    elif stage == "publish":
        with land.Turn(tree):
            land.publish(tree)
    elif stage == "after":
        after(primary, env=env)
    else:
        return cleanup(primary)
    return 0
