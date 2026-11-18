#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A run cut off by its tmux server's end resumes by itself (terminum-dispatcher.md AC-205), end to end: a
private terminumd and tmux server, tasks run by Dispatcher on scripts/fake_agent.py, the server killed.

1. A run at work on a task in progress resumes in a new window: the pool's command with --resume of its
   conversation, the fixed words typed, the task IN PROGRESS, its history saying so.
2. A window closed by hand, the server running on: FAILED, nothing resumed.
3. The server ends again within the hour of that resume: nothing resumed, FAILED with why.

    python3 scripts/e2e_resume.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import BIN, Terminumd, check, end_tmux, summary, tmux_on, wait_for  # noqa: E402
from e2e_remote import api  # noqa: E402
from testrun import free_port, own, short_tmp  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
PORT, SOCKET = free_port(), own("tm-test-resume")
WORDS = ("your session ended because terminum's tmux server stopped (a crash or a restart of the machine), and Dispatcher resumed it. "
         "Your conversation shows what was done: do not redo finished steps.")


class World:
    def __init__(self, work):
        self.work, self.prompts = work, os.path.join(work, "prompts.txt")
        self.cli = os.path.join(work, "bin", "terminum")
        os.makedirs(os.path.dirname(self.cli))
        os.symlink(os.path.realpath(BIN), self.cli)
        os.makedirs(os.path.join(work, "api"))
        # a walk each second (30 s / 30); hooks taken as installed; a finished session not closed under the test
        self.t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1", "TM_TEST_CLOCK_SCALE": "30", "TM_TEST_CLOSE_AFTER": "7200"})
        self.secret = self.project = self.agent = None

    def call(self, path, method="GET", body=None):
        return api(self.t, self.secret, path, method=method, body=json.dumps(body).encode() if body is not None else None,
                   headers={"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"})

    def setup(self):
        self.t.start()
        self.secret = self.t.signin()
        self.project = self.call("/api/task-projects", "POST", {"name": "Api", "dir": os.path.join(self.work, "api")})[1]["id"]
        command = f"python3 -I {os.path.join(HERE, 'fake_agent.py')} {self.cli} --kind claude --prompts {self.prompts}"
        st, added = self.call("/api/task-agents", "POST", {"project": self.project, "name": "Fake", "kind": "claude", "command": command, "at_once": 4})
        check("setup: the agent", st == 200, added)
        self.agent = added["id"]

    def get(self, tid):
        return self.call(f"/api/tasks/{tid}")[1]

    def history(self, tid):
        return " | ".join(e.get("text") or "" for e in self.get(tid).get("history", []))

    def key(self, tid):
        t = self.get(tid)
        return (t.get("session") or {}).get("key") if t.get("status") == "IN PROGRESS" else None

    def running(self, title):
        """A task run on the fake agent, at work (its WORKS prompt keeps a tool open): its ID and session key."""
        tid = self.call("/api/tasks", "POST", {"title": f"{title} WORKS", "project": self.project})[1]["id"]
        st, ran = self.call(f"/api/tasks/{tid}/run", "POST", {"agent": self.agent})
        key = wait_for(lambda: self.key(tid), bool, 30, 0.3, last=True)
        check(f"setup: {tid} runs on its agent", st == 200 and bool(key), ran)
        return tid, key

    def walked(self):
        """Two walks (a second each) saw what stands now."""
        wait_for(lambda: False, bool, 2.5, 0.5, last=True)

    def typed(self):
        try:
            with open(self.prompts, encoding="utf8") as f:
                return f.read()
        except OSError:
            return ""


def main():
    work = os.path.realpath(tempfile.mkdtemp(prefix="tmrs-", dir=short_tmp()))
    w = World(work)
    try:
        w.setup()
        a, a_key = w.running("resumed")
        b, b_key = w.running("closed by hand")
        w.walked()
        # 2. a window closed by hand, its server running on
        tmux_on(SOCKET, "kill-pane", "-t", b_key.rsplit(":", 1)[1])
        check("AC-205 a window closed by hand is no server's end: FAILED, nothing resumed",
              wait_for(lambda: w.get(b).get("status") == "FAILED", bool, 20, 0.5, last=True) and "Its window was closed." in w.history(b)
              and "resumes it" not in w.history(b), w.history(b))
        # 1. the server ends under A's run and is back
        end_tmux(SOCKET)
        # its new window may take the old one's pane ID: a server started anew counts from %0 again
        resumed = wait_for(lambda: "resumes it" in w.history(a) and w.history(a).count("Session started") == 2 and w.key(a), bool, 60, 0.5, last=True)
        check("AC-205 the run resumes in a new window, its task IN PROGRESS, its history saying so",
              bool(resumed) and w.get(a).get("status") == "IN PROGRESS" and "Dispatcher resumes it: its tmux server ended and started anew." in w.history(a), w.history(a))
        check("AC-205 the fixed words with its ID are typed through the gate",
              wait_for(lambda: f"[terminum Dispatcher] {a}: {WORDS}" in w.typed(), bool, 30, 0.5, last=True), w.typed()[-600:])
        pane = (resumed or "").rsplit(":", 1)[-1]
        line = tmux_on(SOCKET, "display", "-p", "-t", pane, "#{pane_start_command}") if pane else ""
        conversation = (w.get(a).get("agent") or {}).get("conversation") or ""
        check("AC-205 the pool's command resumes its conversation", "--resume" in line and bool(conversation) and conversation in line, f"{line} | {conversation}")
        # 3. the server ends again within the hour of that resume
        w.walked()
        end_tmux(SOCKET)
        check("AC-205 a second end within the hour resumes nothing: FAILED with why",
              wait_for(lambda: w.get(a).get("status") == "FAILED", bool, 60, 0.5, last=True)
              and "Its tmux server ended again within an hour of resuming it: run it once the cause is found." in w.history(a), w.history(a))
    except Exception as e:   # a step that cannot go on fails the run with terminumd's last words
        check(f"the scenarios run to their end: {e}", False, w.t.log())
    finally:
        w.t.cleanup()
        shutil.rmtree(work, ignore_errors=True)
    summary()


if __name__ == "__main__":
    main()
