#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Where a release or a rehearsal runs (spec AC-200): a tree of its own, `<primary>/.worktrees/release` or
`.worktrees/rehearse`, detached at the commit it pins, kept between runs (its builds and passes stay warm), so the
primary checkout, the integration queue, Afters and installs go on meanwhile and nothing moves under it.

The pin: PIN= when given (a commit on origin/main); else a rehearsal pins origin/main, and a release the newest commit
on origin/main's first-parent line whose content passed the full run in the last 48 hours, or that differs from such a
commit only in docs (so release notes landed after the nightly's pass are in it), else origin/main itself.
A detached tree is never swept (worktree.py sweep takes branches), and one run holds each tree at a time."""
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_all import FRESH_H, is_gate_doc  # noqa: E402
import gate_record  # noqa: E402
from worktree import fetched, git, link, Refused, trees, Turn  # noqa: E402

BASE = "origin/main"
NEEDS = "scripts/check_all.py"   # a pin older than the release tree cannot run in one


def commits(root, between):
    return git("rev-list", "--count", between, cwd=root).strip()


def default_pin(root, rehearsing):
    """(commit, why it was chosen)."""
    head = git("rev-parse", BASE, cwd=root).strip()
    if rehearsing:
        return head, f"{BASE} as fetched"
    walk = git("rev-list", "--first-parent", "-n", str(gate_record.SCAN_DEPTH), BASE, cwd=root).split()
    for i, commit in enumerate(walk):
        notes = gate_record.passed(root, gate_record.content_of(root, commit))
        if notes is not None and gate_record.young(notes, FRESH_H):
            break
    else:
        return head, f"{BASE}: no commit of it passed the full run in {FRESH_H} hours, so the full run runs whole"
    passed = f"{commit[:9]}'s full pass ({notes.get('at', '?')})"
    for newer in walk[:i]:
        files = git("diff", "--name-only", "--no-renames", commit, newer, cwd=root).split()
        if all(is_gate_doc(f) for f in files):
            return newer, f"docs only since {passed}" + ("" if newer == head else f", {walk.index(newer)} commits behind {BASE}")
    return commit, f"{passed}" + ("" if commit == head else f", {i} commits behind {BASE}")


def pin(root, wanted, rehearsing, say=print):
    """The commit to release: `wanted` resolved, or the default; refused when not on origin/main or too old."""
    if failed := fetched(root, 60):
        say(f"release: warning: git fetch origin: {failed}; {BASE} as last known")
    if wanted:
        run = subprocess.run(["git", "rev-parse", "--verify", "-q", f"{wanted}^{{commit}}"], cwd=root, capture_output=True, text=True)
        if run.returncode:
            raise Refused(f"PIN={wanted} is no commit here")
        commit, why = run.stdout.strip(), f"PIN={wanted}"
    else:
        commit, why = default_pin(root, rehearsing)
    if subprocess.run(["git", "merge-base", "--is-ancestor", commit, BASE], cwd=root).returncode:
        raise Refused(f"{commit[:9]} is not on {BASE}: land it first (a fix for a release lands through the queue, then PIN=<its commit>)")
    if subprocess.run(["git", "cat-file", "-e", f"{commit}:{NEEDS}"], cwd=root, capture_output=True).returncode:
        raise Refused(f"{commit[:9]} predates releases in a tree of their own (it has no {NEEDS}): pin a newer commit")
    passed = gate_record.newest_full_pass(root, commit)
    unchecked = f"{commits(root, f'{passed[0]}..{commit}')} commits since {passed[0][:9]}'s full pass" if passed else "no full pass before it"
    say(f"release: pinned {commit[:9]}: {why}; {unchecked}")
    return commit


def take(primary, name, commit):
    """The kept tree `name` detached at `commit`, made or moved there; refused while it holds uncommitted changes."""
    path = Path(primary) / ".worktrees" / name
    if path in trees(primary):
        if dirty := git("status", "--porcelain", cwd=path).strip():
            raise Refused(f"{path} holds uncommitted changes, nothing of a release's own:\n{dirty}\n(move or remove them first)")
        git("checkout", "--quiet", "--detach", commit, cwd=path)
    else:
        if path.exists():
            raise Refused(f"{path} exists and is no worktree of this repository")
        git("worktree", "add", "--quiet", "--detach", str(path), commit, cwd=primary)
    for line in link(path, primary):
        if line.startswith(("cloned", "linked", "ui/node_modules:", "terminumd/target:")):
            print(f"release: {line}")
    return path


class Held(Turn):
    """One run in each kept tree at a time: the kernel frees it with its holder."""
    def __init__(self, primary, name):
        super().__init__(primary, f"terminum-{name}-tree.turn")
