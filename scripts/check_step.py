#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Runs one check with its own short TMPDIR and TMUX_TMPDIR and fails if it leaves anything there
(spec AC-102): check_all.sh's guard, so a test's leftover fails the run with no false alarm from
another session's. A leftover is a file in the folder, or a process of the check's process group
or working in that folder. Process environments are never queried.
Whatever is left is named, and goes: by its pid, the folder on every exit.

  python3 scripts/check_step.py <command…>"""
import os
import shutil
import signal
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from worktree import working_in  # noqa: E402
from process_cwd import InspectionError  # noqa: E402
import owned_children  # noqa: E402
from check_cancel import Cancellation, unreaped_status  # noqa: E402

SETTLE_S = 15   # what ends by itself gets this long: terminumd's tmutil takes seconds on a busy Mac
LEFT = 3   # the exit status of a check that passed but left something
KILL_AFTER_S = 2   # a leftover still there this long after SIGTERM gets SIGKILL


class GuardCancellation(Cancellation):
    """Notify the pinned private group once, outside the handler, including shell descendants."""
    def __init__(self):
        super().__init__()
        self.forwarded = False

    def forward(self, group):
        if self.number is None or self.forwarded:
            return False
        self.forwarded = True
        try:
            os.killpg(group, self.number)
        except ProcessLookupError:
            pass
        return True


def listing():
    """(pid, pgid, command) of each process but zombies and this one, without environments."""
    listed = subprocess.run(["ps", "-axo", "pid=,pgid=,stat=,command="], capture_output=True, text=True, errors="replace", check=True,
                            start_new_session=True).stdout
    for line in listed.splitlines():
        fields = line.split(None, 3)
        if len(fields) == 4 and fields[0].isdigit() and not fields[2].startswith("Z") and int(fields[0]) != os.getpid():
            yield int(fields[0]), fields[1], fields[3]


class OwnershipError(InspectionError):
    """Directory discovery failed; only the group and registered identities are known."""
    def __init__(self, error, processes):
        super().__init__(str(error))
        self.processes = processes


def own_processes(folder, group):
    """(pid, command) of each process of our private group or working in our private folder."""
    failure = None
    try:
        inside = working_in(folder)
    except InspectionError as error:
        failure, inside = error, ()
    registered = owned_children.active(owned_children.path(folder))
    processes = [(pid, text[:120]) for pid, pgid, text in listing()
                 if pgid == str(group) or pid in inside or pid in registered]
    if failure is not None:
        raise OwnershipError(failure, processes) from failure
    return processes


def send(pid, number):
    """Signal `number` to `pid`; True while it is there (0 only asks): one that ended meanwhile is no error."""
    try:
        os.kill(pid, number)
        return True
    except ProcessLookupError:
        return False


def alive(pid):
    return send(pid, 0)


def end_pids(pids, expected=None):
    """SIGTERM to each, then SIGKILL to those still there KILL_AFTER_S later: an interactive shell
    ignores SIGTERM; one spinning on a terminal gone, SIGHUP too."""
    expected = expected or {}
    def signal_owned(pid, number):
        if pid in expected and owned_children.identity(pid) != expected[pid]:
            return False
        return send(pid, number)
    stays = [pid for pid in pids if signal_owned(pid, signal.SIGTERM)]
    end = time.monotonic() + KILL_AFTER_S
    while stays and time.monotonic() < end:
        time.sleep(0.1)
        stays = [pid for pid in stays if alive(pid)]
    for pid in stays:
        signal_owned(pid, signal.SIGKILL)
    end = time.monotonic() + KILL_AFTER_S
    while stays and time.monotonic() < end:
        listed = subprocess.run(["ps", "-p", ",".join(map(str, stays)), "-o", "pid=,stat="], capture_output=True, text=True)
        if listed.returncode not in (0, 1):   # 1: none of those pids remains
            listed.check_returncode()
        stays = [int(fields[0]) for line in listed.stdout.splitlines()
                 if len(fields := line.split()) == 2 and not fields[1].startswith("Z")]
        if stays:
            time.sleep(0.1)


def files_in(folder):
    """What is left, relative: an empty tmux-<uid> is tmux's own and does not count."""
    try:
        os.rmdir(os.path.join(folder, f"tmux-{os.getuid()}"))
    except OSError:
        pass
    return sorted(os.path.relpath(os.path.join(parent, name), folder)
                  for parent, _, names in os.walk(folder) for name in names or [""]
                  if (parent != folder or names) and os.path.relpath(os.path.join(parent, name), folder) != owned_children.NAME)


def run(command, folder, cancelled, private_group=False):
    """The check's child, status and group; cancellation reaches cleanup even if it ignores signals."""
    env = {**os.environ, "TMPDIR": folder, "TMUX_TMPDIR": folder, "NODE_DISABLE_COMPILE_CACHE": "1", "TM_TEST_CHILDREN_FILE": owned_children.path(folder)}
    child = subprocess.Popen(command, env=env, start_new_session=not private_group)
    owned_children.register(owned_children.path(folder), child.pid)
    group = os.getpgrp() if private_group else child.pid
    while not cancelled.number:
        status = unreaped_status(child)
        if status is not None:
            return child, status, group
        time.sleep(0.1)
    return child, 128 + cancelled.number, group


def main(command, folder=None):
    if not command:
        sys.exit("usage: check_step.py <command…>")
    if folder is not None and os.getpgrp() != os.getpid():
        sys.exit("--owned-root requires a private guard session")
    with GuardCancellation() as cancelled:
        return check(command, folder, cancelled)


def check(command, folder, cancelled):
    private_group = folder is not None
    folder = folder or os.path.realpath(tempfile.mkdtemp(prefix="tmc.", dir="/tmp"))
    inspected = False
    cleaned = False
    try:
        child, status, group = run(command, folder, cancelled, private_group)
        end = time.monotonic() + SETTLE_S
        while True:
            if cancelled.forward(group):
                end = time.monotonic() + SETTLE_S
            try:
                processes = own_processes(folder, group)
                inspected = True
            except OwnershipError as error:
                inspected = False
                processes = error.processes
                print(f"ownership inspection failed: {error}; private root retained: {folder}", file=sys.stderr)
                status = status or 1
                break
            files = files_in(folder)
            if cancelled.number and not cancelled.forwarded:
                continue  # a signal during metadata discovery gets its full grace too
            if not (files or processes) or time.monotonic() >= end:
                break
            time.sleep(1)
        left = len(processes)
        for pid, name in processes:
            print(f"LEFT process {pid}: {name}", file=sys.stderr)
        expected = owned_children.cleanup_identities([pid for pid, _ in processes], owned_children.path(folder))
        end_pids(list(expected), expected)
        child.wait(timeout=KILL_AFTER_S)
        for name in files_in(folder)[:50]:
            left += 1
            print(f"LEFT {name}", file=sys.stderr)
        cleaned = inspected
        return status or (LEFT if left else 0)
    finally:
        if not private_group and cleaned:
            shutil.rmtree(folder, ignore_errors=True)


if __name__ == "__main__":
    args = sys.argv[1:]
    if args[:1] == ["--owned-root"]:
        sys.exit(main(args[2:], folder=args[1]))
    sys.exit(main(args))
