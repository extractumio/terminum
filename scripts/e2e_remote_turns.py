#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""One operation at a host's helper at a time (terminum-remote.md AC-83, AC-99) end to end: a
private terminumd, this Mac playing the host through scripts/fake_ssh.py, each ssh of an
operation at the host's helper (the probe, the copy, the removal, the agents' hooks script) held a
moment (FAKE_SSH_SLOW). Agents on and Files' removal, agents off and Files' enable, agents off and
Files' removal are asked together, in both orders: the second while the first one's ssh runs. At
the end the record is what was asked last, and the helper is on the host exactly when the record
has Files enabled or agents on.

    python3 scripts/e2e_remote_turns.py   (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import stat
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import e2e_ws  # noqa: E402
from e2e_ws import Terminumd, check, summary, unix_request  # noqa: E402
from e2e_remote_agents import STUB, host_rec  # noqa: E402
from testrun import free_port, own  # noqa: E402

PORT, LOCAL_SOCKET, REMOTE_SOCKET = free_port(), own("tm-test-tlocal"), own("tm-test-tremote")
HOLD = 2.0
GATE = "tm-probe-|cat > |rmdir |--host-mode"   # the helper's own connection is never held
ACTIONS = {"agents on": "/hosts/agents-on", "agents off": "/hosts/agents-off",
           "files remove": "/hosts/files-remove", "files enable": "/hosts/enable"}


def record(t):
    """devbox's record in settings.json: (files, agents)."""
    try:
        rec = json.load(open(os.path.join(t.home, "settings.json"))).get("hosts", {}).get("devbox", {})
    except (OSError, ValueError):
        rec = {}
    return rec.get("files"), rec.get("agents")


def on_host(home):
    """(a helper build is in ~/.terminum/bin, terminum-host leads to one) on the host."""
    bin_dir = os.path.join(home, ".terminum", "bin")
    names = os.listdir(bin_dir) if os.path.isdir(bin_dir) else []
    return any(n.startswith("terminumd-agent-") for n in names), os.path.exists(os.path.join(bin_dir, "terminum-host"))


def held(pids, since):
    """The held ssh runs after line `since` of the stand-ins' file."""
    lines = open(pids).read().splitlines() if os.path.exists(pids) else []
    return [x for x in lines[since:] if x.endswith(" held")]


def lines(pids):
    return len(open(pids).read().splitlines()) if os.path.exists(pids) else 0


def ask(t, action):
    st, body = unix_request(t.sock, "POST", ACTIONS[action], {"name": "devbox"})
    return st, (body or {}).get("message") or (body or {}).get("error") or body


def settle(t, first, second, pids, at_host):
    """`first`, then `second` once the first one's ssh at the host runs (`at_host`), else at once; both answers."""
    said, since = {}, lines(pids)
    a = threading.Thread(target=lambda: said.__setitem__(first, ask(t, first)))
    a.start()
    began = e2e_ws.wait_for(lambda: held(pids, since), bool, 30, 0.05, last=True) if at_host else time.sleep(0.05)
    alive = a.is_alive()
    b = threading.Thread(target=lambda: said.__setitem__(second, ask(t, second)))
    b.start()
    a.join()
    b.join()
    if at_host:
        check(f"setup: {second} asked while {first}'s ssh at the host ran", bool(began) and alive, (began, alive))
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
    helper, link = on_host(home)
    want = files == "enabled" or agents
    check(f"AC-83 AC-99 {label}: the record says files {files}, agents {'on' if agents else 'off'}",
          f == files and (a == "enabled") == agents, (f, a))
    check(f"AC-83 AC-99 {label}: the helper is {'on' if want else 'gone from'} the host as the record says, terminum-host {'there' if agents else 'gone'}",
          helper == want and link == agents, os.listdir(os.path.join(home, ".terminum", "bin")) if os.path.isdir(os.path.join(home, ".terminum", "bin")) else None)


CASES = [
    # (from files, from agents, first asked, second asked, files after, agents after, the first has a step at the host)
    ("enabled", False, "agents on", "files remove", "declined", True, True),
    ("enabled", False, "files remove", "agents on", "declined", True, True),
    ("declined", True, "agents off", "files enable", "enabled", False, True),
    ("declined", True, "files enable", "agents off", "enabled", False, True),
    ("enabled", True, "agents off", "files remove", "declined", False, True),
    # a removal of Files while agents are on keeps the helper and only records "declined": no step at the host
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
    pids = os.path.join(t.root, "ssh-pids")
    t.through_fake_ssh(home, REMOTE_SOCKET, pids)
    os.system(f"tmux -L {REMOTE_SOCKET} kill-server 2>/dev/null")
    try:
        t.start()
        st, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "devbox", "recipe": "alex@devbox.example"})
        check("setup: the host is added", st == 200, body)
        up = e2e_ws.wait_for(lambda: host_rec(t), lambda h: h.get("files") == "enabled" and h.get("agents") == "up", 60, 0.3, last=True)
        check("setup: its files and agents are set up by themselves", (up or {}).get("agents") == "up", up)
        t.env["FAKE_SSH_SLOW"] = f"{HOLD}:{GATE}"
        t.stop()
        t.start()
        for files0, agents0, first, second, files1, agents1, at_host in CASES:
            put(t, files0, agents0)
            said = settle(t, first, second, pids, at_host)
            label = f"{first}, then {second}"
            check(f"AC-83 AC-99 {label}: both succeed", all(st == 200 for st, _ in said.values()), said)
            matches(t, home, label, files1, agents1)
    finally:
        t.stop()
        os.system(f"tmux -L {REMOTE_SOCKET} kill-server 2>/dev/null")
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
