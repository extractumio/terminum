#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""What the full runs left (spec AC-201, AC-200): a pass per step and content, kept in the tree and in git's common
folder for every tree of the repository, so a release reuses the nightly's; and one JSON line per full run and per
release or rehearsal in git's common folder, `terminum-gate.jsonl`, which the retry rule reads and the next
post-mortem reads (docs/RELEASING.md).

  gate_record.py [N]      the last N lines (default 10), one a line, for a person
  gate_record.py --nightly   the last rehearsal's red steps, budgets over and retries, as tasks to make (AC-202)"""
import functools
import json
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_changed import content_id  # noqa: E402
from worktree import common_dir, git, Refused  # noqa: E402

RECORD = "terminum-gate.jsonl"
SCAN_DEPTH = 200   # commits of origin/main looked at for the newest full pass
LOST_AFTER, OF_LAST = 3, 7   # a check retried in 3 of the last 7 recorded runs loses its retry


def now():
    return time.strftime("%Y-%m-%d %H:%M")


@functools.lru_cache(maxsize=None)
def common(root):
    """git's common folder of `root`, asked once; None outside a git checkout."""
    try:
        return common_dir(root)
    except Refused:
        return None


def shared(root, name):
    """A file of git's common folder: terminum-checks/<name>; None outside a git checkout."""
    folder = common(Path(root))
    return folder / "terminum-checks" / name if folder else None


def marks(root, ident, step=None):
    """Where a pass of `step` (None: the whole run) on content `ident` is kept: the tree's, then the shared one."""
    name = f"{ident}.ok" if step is None else f"{ident}.{step}.ok"
    return [p for p in (Path(root) / ".checks" / name, shared(root, f"full-{name}")) if p]


def passed(root, ident, step=None):
    """The pass's notes ({"at", "retried", "timing", "skip"}) when one is kept, else None."""
    for mark in marks(root, ident, step):
        try:
            text = mark.read_text()
        except OSError:
            continue
        try:
            notes = json.loads(text)
        except ValueError:   # a pass written before notes: its date alone
            notes = {"at": text.strip()}
        return notes if isinstance(notes, dict) else {"at": str(notes)}
    return None


def keep(root, ident, notes, step=None):
    for mark in marks(root, ident, step):
        mark.parent.mkdir(parents=True, exist_ok=True)
        mark.write_text(json.dumps({"at": now(), **notes}) + "\n")


def forget(root, ident, step=None):
    for mark in marks(root, ident, step):
        mark.unlink(missing_ok=True)


def content_of(root, commit):
    """A clean commit's content id, as check_changed.tree_id gives a clean tree; "" for no commit."""
    run = git("rev-parse", "-q", "--verify", f"{commit}^{{tree}}", cwd=root, check=False)
    return content_id(run.stdout.strip()) if run.returncode == 0 else ""


def first_parents(root, ref):
    """[(commit, its content id)] of `ref`'s first-parent line, newest first, SCAN_DEPTH at most: one git call."""
    run = git("log", "--first-parent", "-n", str(SCAN_DEPTH), "--format=%H %T", ref, "--", cwd=root, check=False)
    return [(c, content_id(t)) for c, t in (line.split() for line in run.stdout.splitlines())] if run.returncode == 0 else []


def young(notes, hours):
    """Whether a pass was kept less than `hours` ago."""
    try:
        return time.time() - time.mktime(time.strptime(notes.get("at", ""), "%Y-%m-%d %H:%M")) < hours * 3600
    except ValueError:
        return False


def newest_full_pass(root, ref="origin/main", fresh_h=None):
    """(commit, its pass's notes) of the newest commit on `ref`'s first-parent line with a full pass (younger than
    `fresh_h` hours when given), or None."""
    for commit, ident in first_parents(root, ref):
        notes = passed(root, ident)
        if notes is not None and (fresh_h is None or young(notes, fresh_h)):
            return commit, notes
    return None


def path(root):
    folder = common(Path(root))
    return folder / RECORD if folder else None


def append(root, entry):
    """One line for one run; a record that cannot be written is said, never fails the run."""
    file = path(root)
    if not file:
        return
    try:
        with file.open("a") as out:
            out.write(json.dumps({"at": now(), **entry}, sort_keys=True) + "\n")
    except OSError as e:
        print(f"record: not written: {e}")


def read(root, kind=None):
    """The record's entries, oldest first; `kind` keeps one kind ("gate", "release", "rehearse")."""
    file = path(root)
    entries = []
    try:
        lines = file.read_text().splitlines() if file else []
    except OSError:
        lines = []
    for line in lines:
        try:
            entry = json.loads(line)
        except ValueError:
            continue
        if isinstance(entry, dict) and (kind is None or entry.get("kind") == kind):
            entries.append(entry)
    return entries


def retried_lately(runs, check):
    """Whether `check` ("e2e_hooks", "page webBoardPanel", "live") was retried in LOST_AFTER of the
    last OF_LAST of the recorded full runs `runs` (read(root, "gate")) that ran its step."""
    step = check.split(" ", 1)[0]
    runs = [run for run in runs if step in run.get("steps", {})][-OF_LAST:]
    return sum(check in run["steps"][step].get("retried", []) for run in runs) >= LOST_AFTER


def nightly(root):
    """For the nightly's agent (AC-202): the last rehearsal's full run, each red step with its cause and the commits
    since the newest full pass before it, one task each."""
    runs = [run for run in read(root, "gate") if run.get("by") == "rehearse"]
    if not runs:
        return ["nightly: no rehearsal's full run in the record"]
    run = runs[-1]
    commit = run.get("commit")
    before = newest_full_pass(root, f"{commit}~1") if commit else None
    since = git("log", "--oneline", f"{before[0]}..{commit}", cwd=root) if before else "(no full pass before it in the record)"
    commit = commit or "HEAD"
    red = {k: s for k, s in run.get("steps", {}).items() if s.get("result") != "pass"}
    lines = [f"nightly: {run.get('result')} on {commit[:9]} at {run.get('at')}, {len(run.get('steps', {}))} steps run"]
    for name, step in red.items():
        lines.append(f"TASK: Bug: the nightly full run is red at {name} on {commit[:9]} (P1): {step.get('cause', 'failed')}"
                     + (f"; crash: {step['crash']}" if step.get("crash") else "") + f"; log .checks/{run.get('content')}.{name}.log;"
                     f" commits since the last full pass:\n{since}")
    for name, step in run.get("steps", {}).items():
        lines += [f"TIMING ({name}): {line}" for line in step.get("timing", [])]
        lines += [f"RETRIED: {check}" for check in step.get("retried", [])]
    return lines


def main(argv):
    if argv == ["--nightly"]:
        print("\n".join(nightly(Path.cwd())))
        return 0
    count = int(argv[0]) if argv else 10
    for entry in read(Path.cwd())[-count:]:
        print(json.dumps(entry, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
