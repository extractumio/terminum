#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-99: cancelling a host operation ends its detached plugin command and descendants.

The real daemon drops an operation's fake ssh; its stdin closes and the production WATCH
ends the nonterminal helper. A held mutation is released only after the opposite operation
answers, exposing a stale install or uninstall that could undo that answer.
"""
import os
from pathlib import Path
import pty
import signal
import subprocess
import threading
import time

from e2e_ws import BIN, Terminumd, check, summary, unix_request, wait_for
from owned_children import identities
from testrun import free_port, own

STUB = r'''#!/usr/bin/env python3
import os
from pathlib import Path
import signal
import subprocess
import sys
import time

home = Path(os.environ["HOME"])
state, gate, blocked = (home / n for n in ("plugin-state", "gate", "blocked"))
if sys.argv[1] == "--mutate":
    # A descendant ignores TERM too: the helper must end the whole detached group.
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    while blocked.exists():
        time.sleep(0.02)
    state.write_text(sys.argv[2])
    sys.exit(0)
args = " ".join(sys.argv[1:])
if args.startswith("plugin install") or args.startswith("plugin uninstall"):
    value = "on" if args.startswith("plugin install") else "off"
    if gate.exists() and args.startswith(gate.read_text()):
        gate.rename(blocked)  # the later operation is never held
        child = subprocess.Popen([sys.executable, __file__, "--mutate", value])
        (home / "started").write_text(f"{os.getpid()} {child.pid}")
        child.wait()
    else:
        state.write_text(value)
sys.exit(0)
'''


def cleanup_children(expected):
    for pid, started in identities(expected).items():
        if expected[pid] == started:
            os.kill(pid, signal.SIGKILL)


def ask(t, action):
    return unix_request(t.sock, "POST", "/hosts/agents-" + action, {"name": "devbox"})


def takeover(t, home, first, second, held):
    started, blocked = home / "started", home / "blocked"
    started.unlink(missing_ok=True)
    (home / "gate").write_text(held)
    answers = []
    worker = threading.Thread(target=lambda: answers.append(ask(t, first)))
    worker.start()
    pids = []
    expected = {}
    try:
        wait_for(started.exists, bool, seconds=30)
        pids = list(map(int, started.read_text().split()))
        expected = identities(pids)
        check(f"AC-99 {first} then {second}: the plugin and its descendant run before takeover",
              worker.is_alive() and len(expected) == 2, pids)
        group = subprocess.run(["ps", "-o", "pgid=", "-p", str(pids[0])], capture_output=True, text=True).stdout.strip()
        check(f"AC-99 {first} then {second}: the plugin leads a group outside the helper's", group == str(pids[0]), group)
        status, body = ask(t, second)
        worker.join(10)
        check(f"AC-99 {first} then {second}: the newer operation answers and the old one is stopped",
              status == 200 and not worker.is_alive() and bool(answers)
              and "took over" in str(answers[0]), (body, answers))
        ended = wait_for(lambda: identities(pids), lambda v: not v,
                         seconds=3, last=True)
        check(f"AC-99 {first} then {second}: closing ssh stdin ends the detached plugin group", not ended, pids)
        blocked.unlink(missing_ok=True)
        # A surviving mutator would now overwrite the newer operation's result.
        wait_for(lambda: identities(pids), lambda v: not v, seconds=5)
        want = "on" if second == "on" else "off"
        got = (home / "plugin-state").read_text()
        check(f"AC-99 {first} then {second}: the final plugin state stays {want} after release", got == want, got)
    finally:
        blocked.unlink(missing_ok=True)
        (home / "gate").unlink(missing_ok=True)
        worker.join(10)
        # The stub records the plugin session leader and its own child's pid; never discover targets.
        cleanup_children(expected)


def direct_signal(t, home, sig, ignored=False, terminal=False):
    """AC-99: the host helper is a nonterminal CLI, not terminumd's graceful server."""
    started, blocked = home / "started", home / "blocked"
    started.unlink(missing_ok=True)
    (home / ".terminum/hooks/claude.installed").unlink(missing_ok=True)
    (home / "gate").write_text("plugin install")
    command = [BIN, "hooks", "install", "--host-mode", "--marker", "TMH-0123456789abcdef",
               "--claude", str(home / ".local/bin/claude")]
    if terminal:
        command = [BIN, "hooks", "install", "claude"]
    if ignored:
        command = ["/bin/sh", "-c", 'trap "" HUP; exec "$@"', "sh", *command]
    env = {**t.env, "HOME": str(home), "TM_HOME": str(home / ".terminum"),
           "PATH": str(home / ".local/bin") + ":" + os.environ["PATH"]}
    master, slave = pty.openpty() if terminal else (None, subprocess.DEVNULL)
    helper = subprocess.Popen(command, env=env, stdin=slave,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    pids = []
    expected = {}
    where = "with a terminal" if terminal else "without a terminal"
    try:
        wait_for(started.exists, bool, seconds=30)
        pids = list(map(int, started.read_text().split()))
        expected = identities(pids)
        helper.send_signal(sig)
        if ignored:
            time.sleep(0.2)
            check("AC-99 an ignored SIGHUP stays ignored without a terminal",
                  helper.poll() is None and len(identities(pids)) == 2)
            blocked.unlink()
            check("AC-99 the helper completes normally after an ignored signal", helper.wait(10) == 0)
        else:
            check(f"AC-99 {signal.Signals(sig).name} {where} ends the helper",
                  helper.wait(10) == -sig)
            remains = wait_for(lambda: identities(pids), lambda v: not v, seconds=3, last=True)
            check(f"AC-99 {signal.Signals(sig).name} {where} ends the plugin and its descendant", not remains)
    finally:
        blocked.unlink(missing_ok=True)
        (home / "gate").unlink(missing_ok=True)
        if helper.poll() is None:
            helper.kill()
        helper.wait()
        cleanup_children(expected)
        if terminal:
            os.close(slave)
            os.close(master)


def main():
    t = Terminumd(free_port(), own("tm-cancel"))
    t.env["HOME"] = t.root
    home = Path(t.root) / "host"
    t.env["CODEX_HOME"] = str(home / ".codex")
    bin_dir = home / ".local/bin"
    bin_dir.mkdir(parents=True)
    (home / ".codex").mkdir()
    stub = bin_dir / "claude"
    stub.write_text(STUB)
    stub.chmod(0o700)
    t.through_fake_ssh(str(home), own("tm-cancel-host"), str(home / "ssh-pids"), TMPDIR=str(home))
    try:
        t.start()
        status, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "devbox", "recipe": "alex@devbox.example"})
        assert status == 200, body
        def ready():
            return unix_request(t.sock, "GET", "/hosts")[1]["hosts"][0].get("agents")
        wait_for(ready, lambda v: v == "up", seconds=120)
        assert ask(t, "off")[0] == 200
        takeover(t, home, "on", "off", "plugin install")
        assert ask(t, "on")[0] == 200
        takeover(t, home, "off", "on", "plugin uninstall")
        for sig in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
            direct_signal(t, home, sig)
        direct_signal(t, home, signal.SIGHUP, ignored=True)
        direct_signal(t, home, signal.SIGINT, terminal=True)
    finally:
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
