# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-102: explicitly owned children, even after setsid, chdir or their parent's exit.

Only a spawner's Popen.pid is registered. Start times prevent a stale record from authorizing
a signal to a reused PID. Neither discovery nor identity checks read process environments.
"""
import json
import os
from pathlib import Path
import subprocess

NAME = ".owned-children"


def path(folder):
    return str(Path(folder) / NAME)


def identities(pids):
    """One metadata query for the recorded PIDs; zombies and ended processes are absent."""
    if not pids:
        return {}
    result = subprocess.run(["ps", "-p", ",".join(map(str, sorted(pids))), "-o", "pid=,lstart=,stat="], capture_output=True, text=True)
    if result.returncode == 1:
        return {}
    result.check_returncode()
    found = {}
    for line in result.stdout.splitlines():
        pid, rest = line.split(None, 1)
        started, state = rest.rsplit(None, 1)
        if not state.startswith("Z"):
            found[int(pid)] = started.strip()
    return found


def identity(pid):
    """A fresh start-identity check immediately before a signal, or None after it ended."""
    return identities([pid]).get(pid)


def register(registry, pid):
    """Record one child immediately after its spawner received Popen.pid."""
    started = identity(pid)
    if started is None:
        return
    line = (json.dumps({"pid": pid, "start": started}) + "\n").encode()
    fd = os.open(registry, os.O_CREAT | os.O_APPEND | os.O_WRONLY, 0o600)
    try:
        if os.write(fd, line) != len(line):
            raise OSError("could not record the child process completely")
    finally:
        os.close(fd)


def records(registry):
    """All registered identities, including ended processes: retain the reuse guard until cleanup."""
    try:
        lines = Path(registry).read_text().splitlines()
    except FileNotFoundError:
        return {}
    result = {}
    for line in lines:
        record = json.loads(line)
        pid, started = record["pid"], record["start"]
        if type(pid) is not int or pid <= 1 or not isinstance(started, str) or not started:
            raise ValueError("invalid owned-child record")
        result[pid] = started
    return result


def active(registry):
    expected = records(registry)
    now = identities(expected)
    return {pid for pid, started in expected.items() if pid != os.getpid() and now.get(pid) == started}
