#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Lands a finished worktree on origin/main (spec AC-104): merged with origin/main in the tree, its
smoke set green on that result, pushed, the primary checkout fast-forwarded and the tree deleted. Its
targeted checks passed on its own content count (scripts/check_changed.py records it); they run again
on the merged result when none is recorded or origin/main changed the same files. Trees land one at a
time: each holds the landing turn from its merge to its push, so one smoke set runs at a time and none is
made moot by another landing's push; a push from outside (a human's) is merged and checked again, still
in the turn (a smoke set ends after SMOKE_MOST_MINUTES). The install runs after the turn, once for all
that landed since the last: by the landing that ends and finds the turn free. A release runs in a tree of its own
(scripts/release_tree.py): nothing here waits for it, and the smoke set yields to a full run (nice).

  land.py <slug>    from the primary checkout (`make land SLUG=<slug>`), the work committed;
                    TM_LAND_INSTALL=1 (the maintainer's local.mk) also installs a change with code. With the
                    project's integration on in terminum, the tree is queued instead (TASK, RESULT)
  land.py <stage> [<slug>]   one of the seven stages alone (scripts/land_stages.py, AC-198)
Exit: 0 landed; 1 checks failed or refused (the tree stays); 2 a merge conflict, left in the tree
to resolve and commit before running this again. No force, ever."""
import json
import os
from pathlib import Path
import re
import signal
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_changed import is_doc, late_notes, red, tree_id  # noqa: E402
import heavy_slots  # noqa: E402
from install_turn import HELD  # noqa: E402
from worktree import common_dir, fetched, INSTALL_TURN, primary_of, Refused, Turn as TreeTurn  # noqa: E402

BASE = "origin/main"
LAND_TURN = "terminum-land.turn"
TRIES = 8
# a smoke set takes about 25 minutes on a busy Mac; one running this long is stuck, and under a held turn
# every other landing would wait for it
SMOKE_MOST_MINUTES = 90
FETCH_MOST_S = 120


class Stop(Exception):
    def __init__(self, code, words):
        super().__init__(words)
        self.code = code


def git(*args, cwd, check=True):
    done = subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True)
    if check and done.returncode:
        raise Stop(1, f"git {' '.join(args)}: {done.stderr.strip() or done.stdout.strip()}")
    return done


def has_base(tree):
    return git("merge-base", "--is-ancestor", BASE, "HEAD", cwd=tree, check=False).returncode == 0


def files_between(a, b, cwd):
    return set(git("diff", "--name-only", f"{a}...{b}", cwd=cwd).stdout.splitlines())


class Turn(TreeTurn):
    """One tree at a time merges, checks and pushes; its holder's name is beside it."""
    def __init__(self, tree):
        try:
            super().__init__(tree, LAND_TURN)
        except Refused as refused:
            raise Stop(1, str(refused)) from refused
        self.slug = Path(tree).name

    def __enter__(self):
        super().__enter__()
        self.take_as(self.slug, lambda holder: print(f"land: {holder}'s landing has its turn (its checks and push can take half an hour): waiting", flush=True))
        return self


def merge_base_in(tree):
    """origin/main merged into the tree; the conflict, if any, stays there to resolve."""
    done = git("merge", "--no-edit", BASE, cwd=tree, check=False)
    if done.returncode:
        conflicts = git("diff", "--name-only", "--diff-filter=U", cwd=tree).stdout.split()
        if not conflicts:
            raise Stop(1, f"merge of {BASE}: {done.stderr.strip() or done.stdout.strip()}")
        raise Stop(2, f"conflict with {BASE} in: {' '.join(conflicts)}\nresolve them in {tree}, commit, and land again")


def update_primary(primary):
    """The primary checkout on the fresh origin/main; one that cannot move is said, never forced."""
    branch = git("branch", "--show-current", cwd=primary).stdout.strip()
    if branch == "main":
        done = git("merge", "--ff-only", BASE, cwd=primary, check=False)
    else:
        done = git("fetch", ".", f"{BASE}:main", cwd=primary, check=False)
    if done.returncode == 0:
        return None
    lines = [line.strip() for line in (done.stderr or done.stdout).splitlines() if line.strip()]
    return "; ".join(line for line in lines if line != "Aborting")[:300] or "git said nothing"


def ci_status(tree, run=subprocess.run):
    """One line on GitHub CI's last verdict on main, as gh says: what a landing adds to; it never stops one. A run
    still waiting or running, or cancelled, is no verdict: one newer than the verdict is said after it."""
    ask = ["gh", "run", "list", "--branch", "main", "--workflow", "ci", "--event", "push", "--limit", "50",
           "--json", "status,conclusion,headSha,url"]
    try:
        done = run(ask, cwd=tree, capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired) as e:
        return f"CI on main: unknown ({e})"
    if done.returncode:
        return f"CI on main: unknown ({(done.stderr.strip().splitlines() or ['gh failed'])[-1]})"
    try:
        runs = json.loads(done.stdout or "[]")
        if not runs:
            return "CI on main: no run"
        said = lambda r: f"{r['conclusion'] or r['status']} on {r['headSha'][:9]}"
        newest = runs[0]
        ended = [r for r in runs if r["status"] == "completed" and r["conclusion"] not in ("cancelled", "skipped")]
        if not ended:
            return f"CI on main: no verdict in the last {len(runs)} runs; newest: {said(newest)}: {newest['url']}"
        last = ended[0]
        then = f"; newer: {said(newest)}" if newest["status"] != "completed" else ""
        verdict = f"CI on main: {said(last)}"
    except (ValueError, KeyError, TypeError, IndexError) as e:
        return f"CI on main: unknown (gh's answer: {e!r})"
    head = git("rev-parse", BASE, cwd=tree).stdout.strip()
    of = "" if last["headSha"] == head else f", not {BASE}'s {head[:9]}"
    return f"{verdict}{of}: {last['url']}{then}"


def checks_of(tree, *flags, minutes=None):
    """Whether the checks passed; ones still running after `minutes` are ended and fail. They run in a process
    group of their own, ended whichever way this returns: each step's scripts/check_step.py takes the signal
    to its own group and cleans up."""
    run = subprocess.Popen([sys.executable, "scripts/check_changed.py", *flags], cwd=tree, start_new_session=True)
    try:
        return run.wait(timeout=minutes and minutes * 60) == 0
    except subprocess.TimeoutExpired:
        print(red(f"land: the checks ran over {minutes} minutes: ended"), flush=True)
        return False
    finally:
        end_group(run)


def end_group(run):
    """SIGTERM to `run`'s group while it lives, SIGKILL 2 minutes later."""
    for number in (signal.SIGTERM, signal.SIGKILL):
        if run.poll() is not None:
            return
        try:
            os.killpg(run.pid, number)
            run.wait(timeout=120)
        except (ProcessLookupError, subprocess.TimeoutExpired):
            pass


def has_code(files):
    return any(not is_doc(f) for f in files)


def tree_state(tree):
    """Both commit identity and file content: even an empty commit changes what may be pushed."""
    return git("rev-parse", "HEAD", cwd=tree).stdout.strip(), tree_id(tree)


def unchanged(tree, expected):
    """Do not adopt concurrent edits, including index-only changes the content hash cannot see."""
    if git("status", "--porcelain", cwd=tree).stdout.strip() or tree_state(tree) != expected:
        raise Stop(1, f"{tree} changed while landing: nothing pushed; commit any changes and land again")


class Moved(Stop):
    """The target moved before a push: prepare and check again."""
    def __init__(self, words):
        super().__init__(1, words)


def head(tree):
    return git("rev-parse", "HEAD", cwd=tree).stdout.strip()


def green_mark(tree, commit):
    """Where a green smoke set of `commit` is kept, in git's common folder beside the landing turn, for every tree of
    the repository: a commit fixes its content, so its green holds wherever it is checked out (AC-198)."""
    return common_dir(tree) / "terminum-land-green" / commit[:16]


def prepare(tree, fetch=True, say=print):
    """Stage 1: committed work, the target merged in (a conflict stays to resolve, exit 2). Whether it merged now;
    done when the target is in HEAD."""
    if git("status", "--porcelain", cwd=tree).stdout.strip():
        raise Stop(1, f"{tree} has uncommitted changes: commit them first")
    # trees of one repository fetch at once (the queue's Verifies, the engine, agents): a held ref is waited for
    if fetch and (failed := fetched(tree, FETCH_MOST_S)):
        raise Stop(1, f"git fetch origin: {failed}")
    if has_base(tree):
        return False
    base_now = git("rev-parse", BASE, cwd=tree).stdout.strip()
    merge_base_in(tree)
    say(f"merged {BASE} ({base_now[:9]})")
    return True


def smoke_green(tree, smoke=lambda tree: checks_of(tree, "--land", minutes=SMOKE_MOST_MINUTES), say=print):
    """Stage 4: the smoke set on HEAD as it is; its green kept for that commit. Done when it is kept."""
    commit = head(tree)
    mark = green_mark(tree, commit)
    if mark.exists():
        say(f"check: {commit[:9]} is green already")
        return
    dirty = lambda: git("status", "--porcelain", cwd=tree).stdout.strip()
    if dirty():
        raise Stop(1, f"{tree} has uncommitted changes: check a commit, not a tree being edited")
    if not smoke(tree):
        raise Stop(1, f"smoke checks failed: {tree} stays with its merge of {BASE}, nothing pushed")
    # the green is that commit's only when the tree held it, unchanged, the whole run
    if head(tree) != commit or dirty():
        raise Stop(1, f"{tree} changed while landing, during its check: nothing marked green, nothing pushed; commit any changes and land again")
    mark.parent.mkdir(parents=True, exist_ok=True)
    mark.write_text(commit + "\n")


def publish(tree, commit=None, say=print):
    """Stage 5: `commit` (HEAD) pushed to the target, never forced, only when it holds the target as it is now and
    was checked green; Moved when the target moved first. Done when the target holds it."""
    git("fetch", "--quiet", "origin", cwd=tree)
    commit = commit or head(tree)
    if git("merge-base", "--is-ancestor", commit, BASE, cwd=tree, check=False).returncode == 0:
        say(f"publish: {commit[:9]} is in {BASE} already")
        return commit
    if not has_base(tree):
        raise Moved(f"{commit[:9]} is not on {BASE} as it is now ({git('rev-parse', BASE, cwd=tree).stdout.strip()[:9]}): prepare and check again")
    if not green_mark(tree, commit).exists():
        raise Stop(1, f"{commit[:9]} has no green check: check first")
    pushed = git("push", "--quiet", "origin", f"{commit}:main", cwd=tree, check=False)
    if pushed.returncode == 0:
        return commit   # the push moved origin/main here too
    git("fetch", "--quiet", "origin", cwd=tree)
    if has_base(tree):
        raise Stop(1, f"push refused, and {BASE} has not moved: {pushed.stderr.strip()}")
    raise Moved(f"{BASE} moved during the push")


def land(tree, check=checks_of, smoke=lambda tree: checks_of(tree, "--land", minutes=SMOKE_MOST_MINUTES),
         passed=lambda tree: checks_of(tree, "--passed"), say=print, after=lambda files: None):
    """The stages in order (AC-198): the commit landed; Stop if it did not land. `after(its files)` runs still in turn."""
    if git("status", "--porcelain", cwd=tree).stdout.strip():
        raise Stop(1, f"{tree} has uncommitted changes: commit them first")
    git("fetch", "--quiet", "origin", cwd=tree)
    if git("merge-base", "--is-ancestor", "HEAD", BASE, cwd=tree, check=False).returncode == 0:
        return head(tree)  # nothing of its own to land
    expected = tree_state(tree)
    # Verify, the tree's own content, before the merge and the turn: no landing waits for it
    if not passed(tree) and not check(tree):
        raise Stop(1, f"targeted checks failed: {tree} stays as it is, nothing pushed")
    own_files = files_between(BASE, "HEAD", tree)

    def once():
        """In the turn: Prepare, Check and Publish once: the commit landed, or None when origin/main moved meanwhile."""
        nonlocal expected
        git("fetch", "--quiet", "origin", cwd=tree)
        unchanged(tree, expected)   # also after waiting for the turn
        ours = files_between(BASE, "HEAD", tree)
        others = files_between("HEAD", BASE, tree)
        if prepare(tree, fetch=False, say=say):
            expected = tree_state(tree)   # only our deliberate merge may replace the checked state
        if ours & others and not check(tree):   # what landed changed its files: verified again on the merge
            raise Stop(1, f"targeted checks failed: {tree} stays with its merge of {BASE}, nothing pushed")
        # each pass follows a merge: a renamed function, a new caller, what others landed meets this tree's code
        smoke_green(tree, smoke, say=say)
        git("fetch", "--quiet", "origin", cwd=tree)
        unchanged(tree, expected)
        if not has_base(tree):
            say(f"{BASE} moved while this tree was checked")
            return None
        try:
            commit = publish(tree, expected[0], say=say)   # HEAD can move even after the last comparison
        except Moved as moved:
            say(str(moved))
            return None
        after(own_files)  # a sweep may take the tree, merged now
        return commit

    # held from the merge to the push: no other landing pushes while this one is checked; only a push from
    # outside moves origin/main, and the next try merges it, still in the turn
    with Turn(tree):
        for _ in range(TRIES):
            commit = once()
            if commit:
                return commit
    raise Stop(1, f"{BASE} kept moving: {TRIES} tries; land again")


def install_mark(primary):
    """A file in git's common folder while code has landed that is not installed yet."""
    return common_dir(primary) / "terminum-land.install"


def after_push(primary, files):
    """In the turn, seconds: the primary checkout on the fresh origin/main; code landed on the maintainer's
    machine (TM_LAND_INSTALL=1) marks an install, run after the turn by install_pending."""
    stuck = update_primary(primary)
    print("main: on the fresh origin/main" if stuck is None else f"primary checkout stays where it is: {stuck}")
    if os.environ.get("TM_LAND_INSTALL") == "1" and has_code(files):
        install_mark(primary).write_text(git("rev-parse", BASE, cwd=primary).stdout)


def make(primary, target):
    """`make <target>` in the primary checkout, its caller holding the install turn: the make install it starts runs
    in that turn instead of waiting for it (scripts/install_turn.py)."""
    return subprocess.run(["make", target], cwd=primary, capture_output=True, text=True, env=dict(os.environ, **{HELD: "1"}))


def landing_holder(primary):
    """Who holds the landing turn, by a look that leaves it free; None when nobody does."""
    with TreeTurn(primary, LAND_TURN) as turn:
        return None if turn.take() else turn.holder()


def install_pending(primary, run=make, say=print):
    """After a landing, landed or not: one install for all code landed since the last, by the landing that finds
    the turn free; one that finds another landing in it leaves the install to it, which comes here after."""
    if not install_mark(primary).exists():
        return
    if holder := landing_holder(primary):
        say(f"install: left to {holder}'s landing, which has the turn")
        return
    with TreeTurn(primary, INSTALL_TURN) as turn:
        # a landing that ended meanwhile, After or one by hand: one after the other
        turn.take_as(f"a landing's install (pid {os.getpid()})", lambda holder: say(f"install: {holder} holds the install turn: waiting for its end"))
        install_now(primary, run, say)


def install_now(primary, run=make, say=print):
    """The install of what the mark names, its caller holding the install turn: in the heavy-build slot, only when the
    primary checkout is on it; the mark removed once installed, unless a later landing wrote its own meanwhile."""
    mark = install_mark(primary)
    try:
        wanted = mark.read_text()
    except FileNotFoundError:
        return   # installed by another
    # the primary checkout is moved only in the turn (after_push), never beside another landing's merge
    if git("merge-base", "--is-ancestor", wanted.strip(), "HEAD", cwd=primary, check=False).returncode:
        say("install: none, the primary checkout is not on what landed: the next landing tries again")
        return
    with heavy_slots.slot(primary, "install", say=say):   # the release build: the heaviest of all
        installed = install_steps(primary, run, say)
    if not installed:
        say("install: the next landing tries again")
    elif mark.read_text() == wanted:
        mark.unlink()   # a later landing's mark stays for it


def install_steps(primary, run, say):
    """make build, install, installed in turn: whether all three passed."""
    for target in ("build", "install", "installed"):
        done = run(primary, target)
        last = (done.stdout.strip().splitlines() or [""])[-1]
        say(f"make {target}: exit {done.returncode}" + (f": {last}" if target == "installed" else ""))
        if done.returncode:
            print(done.stderr.strip()[-600:], file=sys.stderr)
            return False
    return True


def main(argv):
    sys.modules.setdefault("land", sys.modules[__name__])   # run as a script, this is the land land_stages imports
    import land_stages   # the stages outside the turn and their command line
    slug_ok = lambda a: re.fullmatch(r"[A-Za-z0-9][\w.-]*", a)
    staged = bool(argv) and argv[0] in land_stages.STAGES
    if not argv or len(argv) > (2 if staged else 1) or not all(slug_ok(a) for a in argv):
        sys.exit("usage: land.py <slug> | land.py <stage> [<slug>], a stage one of " + ", ".join(land_stages.STAGES))
    try:
        primary = primary_of(Path.cwd())
    except Refused as refused:
        sys.exit(str(refused))
    tree = None if staged else primary / ".worktrees" / argv[0]
    if tree and not tree.is_dir():
        sys.exit(f"no worktree {tree}")
    try:
        if staged:
            return land_stages.run_stage(argv[0], argv[1] if len(argv) > 1 else None, primary)
        # the project's integration on: terminum runs the stages; this tree is queued, never pushed from here
        if land_stages.integration_on(tree, os.environ.get("TASK")):
            return land_stages.queue(tree, os.environ.get("TASK"), os.environ.get("RESULT"), on=lambda *_: True)
    except Stop as stop:
        print(f"land: {stop}", file=sys.stderr)
        return stop.code
    git("fetch", "--quiet", "origin", cwd=tree, check=False)   # main's head as GitHub has it, for ci_status
    ci = ci_status(tree)
    print(ci)   # said again last when not green: minutes of checks scroll the first out of sight
    green = ci.startswith("CI on main: success")
    late, stopped = [], None

    def after(files):   # read now: once merged, a sweep may take the tree
        late.append(late_notes(tree))
        after_push(primary, files)
    try:
        commit = land(tree, after=after)
    except Stop as stop:
        stopped = stop
    install_pending(primary)   # landed or not: this landing's, or one left to it
    if stopped:
        print(f"land: {stopped}", file=sys.stderr)
        if not green:
            print(ci)
        return stopped.code
    print(f"landed {commit[:9]} on {BASE}")
    if not green:
        print(ci)
    if late and late[0]:
        print(red(late[0]))   # timed checks skipped or passed only on retry: the report says so
    os.chdir(primary)
    if not tree.is_dir():
        return 0
    gone = subprocess.run([str(primary / "scripts" / "delete_worktree.sh"), argv[0]])
    if gone.returncode:
        print(f"{tree} stays: see above; the Stop hook's sweep takes it when it is free", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
