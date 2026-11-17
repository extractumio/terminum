#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""`make release` and `make rehearse` (spec AC-200, AC-104): from any checkout, a commit pinned and a tree of its own
detached at it (scripts/release_tree.py), then in that tree, by its own code (`make release-here`, `rehearse-here`):
leftovers cleared, the full run (scripts/check_all.py, which reuses the passes kept for that content), the Mac test
runner's checks (a pass kept per content too), then scripts/release.py, which builds, signs, tags the pin and
publishes. Failing tests, here or on the Mac test runner, stop a release; a problem clearing leftovers, or a runner
that could not run them, is said. The full run starts on a quiet machine, one at a time (scripts/load_gate.py,
AC-146); a release finding a rehearsal's full run in the turn waits for its end and reuses its passes. `--rehearse`
stops before anything is signed or public. Every run leaves a line in the record (scripts/gate_record.py)."""
import argparse
import os
from pathlib import Path
import re
import signal
import subprocess
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_changed import tree_id  # noqa: E402
import gate_record  # noqa: E402
import load_gate  # noqa: E402
import release  # noqa: E402
import release_tree  # noqa: E402
from worktree import primary_of, Refused  # noqa: E402

MAC_CAP_S = 3600   # a runner gone quiet must not hold a release
BUSY_WAIT_S, BUSY_RETRY_S = 1800, 60
# ci_mac.sh's exits that are no failing test, what each says
COULD_NOT_RUN = {0: "", 2: "the runner could not run them", 3: "the runner is busy, or its lock is stale"}


def mac_checks(repo):
    """The checks on the Mac test runner (scripts/ci_mac.sh): (failing tests, why they did not run)."""
    if not os.environ.get("TM_MAC_RUNNER"):
        return False, "no TM_MAC_RUNNER"
    busy_until = time.monotonic() + BUSY_WAIT_S
    while True:
        run = subprocess.Popen(["bash", "scripts/ci_mac.sh"], cwd=repo, start_new_session=True)
        try:
            status = run.wait(MAC_CAP_S)
        except subprocess.TimeoutExpired:
            os.killpg(run.pid, signal.SIGTERM)   # its ssh too
            run.wait()
            return False, f"it took over {MAC_CAP_S // 60} minutes"
        if status != 3 or time.monotonic() >= busy_until:   # another run, as a rehearsal, may just be ending
            return status not in COULD_NOT_RUN, COULD_NOT_RUN.get(status, "")
        time.sleep(BUSY_RETRY_S)


def chain(repo, tag, rehearsing, say=print):
    """In the pinned tree: leftovers, a quiet machine, the full run, the Mac test runner's, then scripts/release.py."""
    by, began = ("rehearse" if rehearsing else "release"), time.monotonic()
    head, ident = release.run("git", "rev-parse", "HEAD", cwd=repo), tree_id(repo)
    entry = {"kind": by, "tag": tag, "commit": head, "content": ident}

    def ended(result, code):
        gate = (gate_record.read(repo, "gate") or [{}])[-1]
        if gate.get("content") == ident:
            entry["red"] = {k: s.get("cause", "failed") for k, s in gate.get("steps", {}).items() if s.get("result") != "pass"}
        entry.update(result=result, seconds=round(time.monotonic() - began))
        gate_record.append(repo, entry)
        return code

    subprocess.run([sys.executable, "scripts/leftovers.py", "--end"], cwd=repo)
    # its turn, a quiet machine, the passes it can reuse: the full run's own (AC-146, AC-201)
    status = subprocess.run([sys.executable, "scripts/check_all.py", "--by", by], cwd=repo).returncode
    if status == load_gate.BUSY:
        say(f"release: the build server is not available for a release: {load_gate.MINUTES}: nothing is public", file=sys.stderr)
        return ended("busy", 1)
    if status:
        say("release: failing tests stop it: nothing is public", file=sys.stderr)
        return ended("red: the full run", 1)
    retried = (gate_record.passed(repo, ident) or {}).get("retried", [])
    entry["retried"] = retried
    if gate_record.passed(repo, ident, "mac"):
        failed, why = False, ""
        say("release: the Mac test runner's checks passed on this content already")
    else:
        failed, why = mac_checks(repo)
        if not failed and not why:
            gate_record.keep(repo, ident, {}, "mac")
    entry["mac"] = "red" if failed else why or "pass"
    if failed:
        say("release: failing tests on the Mac test runner stop it: nothing is public", file=sys.stderr)
        return ended("red: the Mac test runner", 1)
    said = f"the Mac test runner's checks did not run: {why}" if why else ""
    try:
        release.main(tag, head, said, rehearsing, retried)
    except SystemExit as stop:
        ended(f"stopped: {stop.code}", 1)
        raise
    finally:
        if said:
            say(f"release: {said}")
    return ended("rehearsed" if rehearsing else "published", 0)


def say(*words, file=sys.stdout):
    print(*words, file=file, flush=True)


def here(tag, rehearsing):
    """make release-here / rehearse-here: the chain in the pinned tree, by its own code."""
    repo = Path(__file__).resolve().parent.parent
    if primary_of(repo) == repo:   # its build replaces ui/dist and dist/package under other sessions
        sys.exit("release: the chain runs in its pinned tree: make release (or make rehearse) pins one")
    release.check_settings()   # before the long run: none of these is any test's fault
    release.check_source(tag)
    if not rehearsing:
        release.release_notes(tag)
    return chain(repo, tag, rehearsing, say)


def main(argv):
    parser = argparse.ArgumentParser(prog="release_turn.py")
    parser.add_argument("--rehearse", action="store_true")
    parser.add_argument("--here", action="store_true")
    parser.add_argument("--pin")
    parser.add_argument("tag")
    args = parser.parse_args(argv)
    if not re.fullmatch(r"v[0-9]+\.[0-9]+\.[0-9]+", args.tag):
        sys.exit("usage: release_turn.py [--rehearse] [--pin <commit>] vX.Y.Z")
    if args.here:
        return here(args.tag, args.rehearse)
    primary = primary_of(Path(__file__).resolve().parent)
    name = "rehearse" if args.rehearse else "release"
    try:
        wanted = args.pin or (f"refs/tags/{args.tag}" if release.tag_exists(args.tag) else None)   # released where it stands
        commit = release_tree.pin(primary, wanted, args.rehearse, say)
        with release_tree.Held(primary, name) as held:
            if not held.take():
                raise Refused(f"another {name} runs in .worktrees/{name}: {held.holder()}")
            held.hold_as(f"{name} {args.tag} at {commit[:9]}, pid {os.getpid()}")
            tree = release_tree.take(primary, name, commit)
            say(f"release: in {tree}, detached at {commit[:9]}")
            return subprocess.run(["make", "--no-print-directory", f"{name}-here", f"TAG={args.tag}"], cwd=tree).returncode
    except Refused as refused:
        sys.exit(f"release: {refused}")


if __name__ == "__main__":
    sys.stdout.reconfigure(line_buffering=True)   # make's log is often a file: a gate's wait is seen while it waits
    sys.exit(main(sys.argv[1:]))
