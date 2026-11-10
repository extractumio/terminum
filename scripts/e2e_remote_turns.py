#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""One operation at a host's helper at a time (terminum-remote.md AC-83, AC-99) end to end: a
private terminumd, this Mac playing the host through scripts/fake_ssh.py. Agents on and Files'
removal, agents off and Files' enable, agents off and Files' removal are asked together, in both
orders: the first one's ssh at the host is held (FAKE_SSH_HOLD) until the second is asked. No copy,
removal or hooks script at the host overlaps another (FAKE_SSH_RUNS), the record ends as asked, and
the helper is on the host exactly when the record has Files enabled or agents on.

    python3 scripts/e2e_remote_turns.py   (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import re
import stat
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_ws  # noqa: E402
from e2e_ws import Terminumd, check, end_tmux, summary, unix_request  # noqa: E402
from e2e_remote_agents import STUB, host_rec  # noqa: E402
from testrun import free_port, own  # noqa: E402

PORT, LOCAL_SOCKET, REMOTE_SOCKET = free_port(), own("tm-test-tlocal"), own("tm-test-tremote")
# the ssh runs of an operation at the helper: the probe, the copy, the removal, the agents' hooks
# script; never Files' tunnel or the agents' link, which reconnect by themselves meanwhile
GATE = "tm-probe-|cat > |rmdir |--host-mode"
# of those, the steps that change the host: the copy, the removal, the hooks script
STEPS = re.compile(r"cat > |rmdir |--host-mode")
ACTIONS = {"agents on": "/hosts/agents-on", "agents off": "/hosts/agents-off",
           "files remove": "/hosts/files-remove", "files enable": "/hosts/enable"}


def record(t):
    """devbox's record in settings.json (the stored "agents", not the link's state): (files, agents)."""
    rec = json.load(open(os.path.join(t.home, "settings.json")))["hosts"]["devbox"]
    return rec.get("files"), rec.get("agents")


def ask(t, action):
    st, body = unix_request(t.sock, "POST", ACTIONS[action], {"name": "devbox"})
    return st, (body or {}).get("message") or (body or {}).get("error") or body


def quiet(pids):
    """Waits until no ssh run started for 1.5 s: a probe of a helper's connection reconnecting after the
    last step is not taken for the first operation's."""
    size, still = -1, time.time()
    end = time.time() + 30
    while time.time() < end and time.time() - still < 1.5:
        now = os.path.getsize(pids) if os.path.exists(pids) else 0
        if now != size:
            size, still = now, time.time()
        time.sleep(0.1)


def overlapping(runs, since):
    """(the host-changing ssh runs that started after `since`, the pairs of them that overlapped in time)."""
    steps = []
    for line in open(runs) if os.path.exists(runs) else []:
        pid, start, end, command = line.split(" ", 3)
        if float(start) >= since and STEPS.search(command):
            steps.append((float(start), float(end), command[:60]))
    steps.sort()
    return len(steps), [(a[2], b[2]) for i, a in enumerate(steps) for b in steps[i + 1:] if b[0] < a[1]]


def settle(t, first, second, flag, at_host):
    """`first`, then `second` while the first one's ssh at the host is held (`at_host`), else at once; both answers."""
    said = {}

    def go(action):
        th = threading.Thread(target=lambda: said.__setitem__(action, ask(t, action)))
        th.start()
        return th

    if at_host:
        open(flag, "w").write(GATE + "\n")
    a = go(first)
    if at_host:
        held = e2e_ws.wait_for(lambda: open(flag).read().splitlines()[1:], bool, 30, 0.05, last=True)
    else:
        time.sleep(0.05)
    alive = a.is_alive()
    b = go(second)
    if at_host:
        time.sleep(0.5)   # the second reaches terminumd and waits for its turn
        os.unlink(flag)
        check(f"setup: {second} asked while {first}'s ssh at the host ran", bool(held) and alive, (held, alive))
    a.join()
    b.join()
    return said


def put(t, files, agents):
    """Brings devbox to files `files` ("enabled"/"declined") and agents `agents` (True/False), one at a time."""
    if (record(t)[0] == "enabled") != (files == "enabled"):
        st, body = ask(t, "files enable" if files == "enabled" else "files remove")
        check(f"setup: files {files}", st == 200, body)
    if (record(t)[1] == "enabled") != agents:
        st, body = ask(t, "agents on" if agents else "agents off")
        check(f"setup: agents {'on' if agents else 'off'}", st == 200, body)


def matches(t, home, label, files, agents):
    """After both answered: the record is `files`, `agents`, and the host has the helper exactly then."""
    f, a = record(t)
    bin_dir = os.path.join(home, ".terminum", "bin")
    names = sorted(os.listdir(bin_dir)) if os.path.isdir(bin_dir) else []
    helper, link = any(n.startswith("terminumd-agent-") for n in names), os.path.lexists(os.path.join(bin_dir, "terminum-host"))
    want = files == "enabled" or agents
    check(f"AC-83 AC-99 {label}: the record says files {files}, agents {'on' if agents else 'off'}",
          f == files and (a == "enabled") == agents, (f, a))
    check(f"AC-83 AC-99 {label}: the helper is {'on' if want else 'gone from'} the host as the record says, terminum-host {'there' if agents else 'gone'}",
          helper == want and link == agents, names)


# (from files, from agents, first asked, second asked, files after, agents after, the first has a
# step at the host); each pair starts where the one before ended where it can
CASES = [
    ("enabled", False, "agents on", "files remove", "declined", True, True),
    ("declined", True, "agents off", "files enable", "enabled", False, True),
    ("enabled", False, "files remove", "agents on", "declined", True, True),
    ("declined", True, "files enable", "agents off", "enabled", False, True),
    ("enabled", True, "agents off", "files remove", "declined", False, True),
    # sequential, not a race: a removal of Files while agents are on keeps the helper and only records
    # "declined", with no step at the host to hold; that it judges in its turn the first case shows
    ("enabled", True, "files remove", "agents off", "declined", False, False),
]


def main():
    t = Terminumd(PORT, LOCAL_SOCKET)
    home = os.path.join(t.root, "rh")
    os.makedirs(os.path.join(home, ".local", "bin"))
    os.makedirs(os.path.join(home, ".codex"))
    stub = os.path.join(home, ".local", "bin", "claude")
    open(stub, "w").write(STUB)
    os.chmod(stub, stat.S_IRWXU)
    pids, flag, runs = (os.path.join(t.root, n) for n in ("ssh-pids", "hold", "runs"))
    t.through_fake_ssh(home, REMOTE_SOCKET, pids, FAKE_SSH_HOLD=flag, FAKE_SSH_RUNS=runs)
    try:
        t.start()
        st, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "devbox", "recipe": "alex@devbox.example"})
        check("setup: the host is added", st == 200, body)
        up = e2e_ws.wait_for(lambda: host_rec(t), lambda h: h.get("files") == "enabled" and h.get("agents") == "up", 60, 0.3, last=True)
        check("setup: its files and agents are set up by themselves", (up or {}).get("agents") == "up", up)
        for files0, agents0, first, second, files1, agents1, at_host in CASES:
            put(t, files0, agents0)
            quiet(pids)
            since = time.time()
            said = settle(t, first, second, flag, at_host)
            label = f"{first}, then {second}"
            check(f"AC-83 AC-99 {label}: both succeed", all(st == 200 for st, _ in said.values()), said)
            n, both = overlapping(runs, since)
            check(f"AC-83 AC-99 {label}: no copy, removal or hooks script at the host overlapped another ({n} ran)", n >= 2 and not both, both)
            matches(t, home, label, files1, agents1)
    finally:
        if os.path.exists(flag):
            os.unlink(flag)
        t.stop()
        end_tmux(REMOTE_SOCKET)
        if os.path.exists(pids):
            for line in open(pids):
                try:
                    os.kill(int(line.split()[0]), 15)
                except (ProcessLookupError, ValueError):
                    pass
        if "--keep" not in sys.argv:
            t.cleanup()
    summary()


if __name__ == "__main__":
    main()
