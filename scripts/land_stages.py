#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The seven stages of a landing (docs/specs/terminum-integration.md AC-198), each a command of its own, each
idempotent: run again it does nothing when its work is done, and goes on otherwise. terminumd's integration queue
runs them by a project's settings; without it `land.py <slug>` runs them in order: land() of scripts/land.py, which
holds Prepare, Check and Publish in the landing turn.

  land.py prepare [<slug>]   the target merged into the tree; a conflict stays there to resolve (exit 2)
  land.py verify  [<slug>]   the targeted checks on the tree's content (kept per content), or the pass of the commit
                             Prepare merged into when origin/main changed none of its files; with
                             TM_INTEGRATION_RERUN, a red Verify's failed steps run on the target
  land.py queue   <slug>     with integration on: queued by terminum (TASK, RESULT); off: nothing to queue
  land.py check   [<slug>]   the smoke set on HEAD, its green kept for that commit
  land.py publish <slug>     pushed, never forced, only a commit checked green that holds the target as it is now
  land.py after              the primary checkout on the fresh target; with TM_LAND_INSTALL=1, this build installed
  land.py cleanup            landed trees removed by worktree.py sweep, never one with work or a process in it

With no <slug> a stage runs in the folder it is started in (terminumd's own trees). Exit: 0 done; 1 failed or
refused; 2 a conflict left in the tree; 75 (After) it cannot run now, terminumd's queue runs it again (AC-191)."""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import land  # noqa: E402
from land import Stop, git  # noqa: E402
from worktree import INSTALL_TURN, Turn  # noqa: E402

STAGES = ("prepare", "verify", "queue", "check", "publish", "after", "cleanup")


def terminum():
    """The terminum command: on PATH, else where its install puts it."""
    return shutil.which("terminum") or str(Path.home() / ".terminum" / "bin" / "terminum")


def verify(tree, check=land.checks_of, say=print, env=os.environ):
    """Stage 2: the targeted checks on the tree's content; check_changed.py itself skips a pass kept for it, or for
    the commit Prepare merged the target into when the target changed none of its files (AC-198). terminumd's run of
    a red Verify's failed steps on the target (TM_INTEGRATION_RERUN, of TM_INTEGRATION_RERUN_OF) runs only those."""
    steps, of = env.get("TM_INTEGRATION_RERUN"), env.get("TM_INTEGRATION_RERUN_OF")
    flags = ("--rerun", steps, "--of", of) if steps and of else ()
    if not check(tree, *flags):
        raise Stop(1, f"verify: targeted checks failed in {tree}")
    say("verify: passed")


# what `terminum` says when terminumd does not run (cli/mcp.rs DOWN): the stages then run here, in order
DOWN = "terminum is not running"


def integration_on(tree, task=None, run=None):
    """Whether terminum queues the landings of task `task`'s project (else the calling session's): its
    integration_status. None when terminum is not running or not installed: the stages run here, in order. Any
    other answer it cannot read refuses: a landing never pushes from the tree when integration may be on."""
    ask = json.dumps({"id": task} if task else {})
    try:
        done = (run or subprocess.run)([terminum(), "tool", "integration_status", ask], cwd=tree, capture_output=True, text=True, timeout=30)
    except FileNotFoundError:
        return None
    except (OSError, subprocess.TimeoutExpired) as e:
        raise Stop(1, f"whether integration is on is not known ({e}): nothing pushed") from e
    said = f"{done.stdout or ''} {done.stderr or ''}"
    if done.returncode and DOWN in said:
        return None
    try:
        if done.returncode:
            raise ValueError(said.strip())
        return bool(json.loads(done.stdout)["on"])
    except (ValueError, KeyError, TypeError) as e:
        raise Stop(1, f"whether integration is on is not known: {e}: nothing pushed; give TASK=<ID> from a session of terminum") from e


def from_terminumd(env=os.environ):
    """Whether terminumd's queue runs this stage: its commands get the batch's variables (§9)."""
    return bool(env.get("TM_INTEGRATION_BATCH") or env.get("TM_INTEGRATION_TASK"))


def not_while_queued(tree, stage, env=os.environ, on=None):
    """Publish and After move what terminumd's queue moves: by hand they are refused while integration is on."""
    if not from_terminumd(env) and (on or integration_on)(tree, env.get("TASK")):
        raise Stop(1, f"{stage}: integration is on: terminumd publishes and runs After; queue with make land TASK=<ID> RESULT=<file>")


def queue(tree, task=None, result=None, on=integration_on, run=None, say=print):
    """Stage 3: with integration on, the tree's HEAD queued by terminum for task `task` with its result file, and
    waited for (exit 0 landed, 1 red, 2 conflict, 3 taken out); off, nothing to queue: check and publish take the
    landing turn."""
    if not on(tree, task):
        say("queue: integration is off: the landing turn orders check and publish")
        return 0
    if not task or not result:
        raise Stop(1, "queue: integration is on: give TASK=<ID> and RESULT=<file with the result finish_task "
                      "would take>, or queue from the session with integrate {id, result}")
    if git("status", "--porcelain", cwd=tree).stdout.strip():
        raise Stop(1, f"{tree} has uncommitted changes: commit them first")
    # read from where make land was run, not from the tree the queue runs in
    argv = [terminum(), "integration", "queue", task, "--result-file", str(Path(result).resolve()), "--commit", land.head(tree), "--wait"]
    return (run or subprocess.run)(argv, cwd=tree).returncode


# EX_TEMPFAIL: the stage cannot run now; terminumd's queue runs it again after a pause (AC-191)
LATER = 75


def after(primary, run=land.make, env=os.environ, say=print):
    """Stage 6: the primary checkout on the fresh target; on the maintainer's machine (TM_LAND_INSTALL=1) this build
    installed, by the landings' own install (its heavy-build slot, its guard that the checkout is on what landed).
    Done when the checkout follows and `make installed` says this build runs. It cannot run now, and moves nothing,
    while another install or a landing holds its turn: LATER. It holds the install turn throughout."""
    with Turn(primary, INSTALL_TURN) as turn:
        if not turn.take():
            raise Stop(LATER, "after: an install runs in the primary checkout: waits for its end")
        turn.hold_as(f"a landing's After (pid {os.getpid()})")
        if land.landing_holder(primary):
            raise Stop(LATER, "after: a landing holds its turn: waits for its end")
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
        land.install_now(primary, run=run, say=say)
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
        not_while_queued(tree, stage, env)
        with land.Turn(tree):
            land.publish(tree)
    elif stage == "after":
        not_while_queued(primary, stage, env)
        after(primary, env=env)
    else:
        return cleanup(primary)
    return 0
