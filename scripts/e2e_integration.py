#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Integration's stages across a restart of terminumd and an end of terminum's tmux server
(terminum-integration.md AC-187, AC-190, AC-191, AC-198), end to end: a private terminumd, a private
tmux server, a private bare remote; each task run by Dispatcher on scripts/fake_agent.py and queued
by its own pane, as an agent's `terminum integration queue`; each stage a stand-in line that logs
its start and waits while a hold file of the test stands.

1. A Check outlives a restart of terminumd (AC-190).
2. terminumd restarts while an After and the next batch's Check run (AC-191).
3. The tmux server ends while a Verify, a Check and an After run at once: Verify and Check run
   again, the After does not and its row says so; nothing but a green candidate is published;
   every queued task stays queued (AC-187, AC-190, AC-191, AC-198).
4. The tmux server ends while the project's Publish runs, after its push: the batch is published
   once, never forced (AC-191).

    python3 scripts/e2e_integration.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import BIN, Terminumd, check, end_tmux, summary, tmux_on, wait_for  # noqa: E402
from e2e_remote import api  # noqa: E402
from testrun import free_port, own, short_tmp  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
PORT, SOCKET = free_port(), own("tm-test-integ")
tmux = partial(tmux_on, SOCKET)
# Logs "<name> start" and "<name> end" to runs.log; waits while hold-<name> stands; ends 0.
STAGE = """#!/bin/sh
ctl=$(dirname "$0")
echo "$1 start" >> "$ctl/runs.log"
while [ -e "$ctl/hold-$1" ]; do sleep 0.2; done
echo "$1 end" >> "$ctl/runs.log"
"""
# the result in its sections, as integrate takes it (terminumd/src/tasks/closeout.rs SECTIONS)
RESULT = "Outcome: Done, committed on its tree.\n" + "\n".join(f"{h}: None." for h in
    ("Done", "Checked", "Not done", "Needs a human", "New tasks", "Tasks changed", "Issues and discoveries", "Further recommendations"))
SLOW = 150   # a step of the queue: a beat each (TM_TEST_INTEGRATION_BEAT), a stage line's start, a batch built, checked, published


def git(folder, *args):
    return subprocess.run(["git", "-C", folder, "-c", "user.name=t", "-c", "user.email=t@example.com", *args], capture_output=True, text=True, check=True).stdout.strip()


class World:
    """The test's terminumd, its project and agent, the remote and the stage lines' control folder."""

    def __init__(self, work):
        self.work, self.ctl = work, os.path.join(work, "ctl")
        self.origin, self.repo = os.path.join(work, "origin.git"), os.path.join(work, "billing")
        os.makedirs(self.ctl)
        with open(os.path.join(self.ctl, "stage.sh"), "w") as f:
            f.write(STAGE)
        subprocess.run(["git", "init", "-q", "--bare", "-b", "main", self.origin], check=True)
        subprocess.run(["git", "clone", "-q", self.origin, self.repo], check=True, capture_output=True)
        with open(os.path.join(self.repo, "base.txt"), "w") as f:
            f.write("base\n")
        git(self.repo, "add", "base.txt")
        git(self.repo, "commit", "-qm", "base")
        git(self.repo, "push", "-q", "origin", "HEAD:main")
        git(self.repo, "fetch", "-q", "origin")
        self.cli = os.path.join(work, "bin", "terminum")
        os.makedirs(os.path.dirname(self.cli))
        self.t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1", "TM_TEST_INTEGRATION_BEAT": "1"})   # hooks taken as installed: Dispatcher runs the fake agent
        os.symlink(os.path.realpath(BIN), self.cli)
        self.secret = self.project = self.agent = None

    def start(self, keep_tmux=False):
        self.t.start(keep_tmux)
        self.secret = self.t.signin()

    def restart(self):
        """terminumd stopped and started again, its tmux server going on, as an After's install does."""
        self.t.stop()
        self.start(keep_tmux=True)

    def call(self, path, method="GET", body=None):
        return api(self.t, self.secret, path, method=method, body=json.dumps(body).encode() if body is not None else None,
                   headers={"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"})

    def stage(self, kind, ident):
        return f"sh {self.ctl}/stage.sh {kind}-{ident}"

    def setup(self):
        self.project = self.call("/api/task-projects", "POST", {"name": "Billing", "dir": self.repo})[1]["id"]
        command = f"python3 -I {os.path.join(HERE, 'fake_agent.py')} {self.cli} --kind claude"
        st, added = self.call("/api/task-agents", "POST", {"project": self.project, "name": "Fake", "kind": "claude", "command": command, "at_once": 8})
        check("setup: the agent", st == 200, added)
        self.agent = added["id"]
        self.call(f"/api/task-projects/{self.project}/agents-at-once", "PUT", {"limit": 8})
        self.settings(publish="")

    def settings(self, publish):
        st, said = self.call(f"/api/task-projects/{self.project}/integration", "PUT", {
            "on": True, "verify": self.stage("verify", "$TM_INTEGRATION_TASK"), "check": self.stage("check", "$TM_INTEGRATION_BATCH"),
            "after": self.stage("after", "$TM_INTEGRATION_BATCH"), "publish": publish})
        check("setup: integration on with stand-in lines", st == 200 and said.get("ok"), said)

    # --- tasks ---
    def task(self, letter):
        return self.call("/api/tasks", "POST", {"title": f"{letter} WORKS", "project": self.project})[1]["id"]

    def queue(self, tid, letter):
        """Its commit in a tree of its own; Dispatcher's run of it on the fake agent; queued by that run's pane."""
        tree = os.path.join(self.work, "trees", letter)
        git(self.repo, "worktree", "add", "-q", "--detach", tree, "origin/main")
        with open(os.path.join(tree, f"{letter}.txt"), "w") as f:
            f.write(f"{letter}\n")
        git(tree, "add", f"{letter}.txt")
        git(tree, "commit", "-qm", letter)
        sha = git(tree, "rev-parse", "HEAD")
        st, ran = self.call(f"/api/tasks/{tid}/run", "POST", {"agent": self.agent})
        def taken():
            t = self.get(tid)
            return (t.get("session") or {}).get("key") if t.get("status") == "IN PROGRESS" else None
        key = wait_for(taken, bool, 30, 0.3, last=True)
        check(f"setup: {tid} runs on its agent", st == 200 and bool(key), ran)
        pane = key.rsplit(":", 1)[1]
        result = os.path.join(self.work, f"result-{letter}.md")
        with open(result, "w") as f:
            f.write(RESULT)
        env = {**os.environ, "TM_HOME": self.t.home, "TM_APP_DIR": self.t.app, "TMUX_PANE": pane,
               "TMUX": tmux("display", "-p", "#{socket_path},#{pid},0").strip()}
        q = subprocess.run([self.cli, "integration", "queue", tid, "--result-file", result, "--commit", sha], env=env, capture_output=True, text=True, timeout=30)
        check(f"setup: {tid} is queued by its own pane", q.returncode == 0 and self.status(tid) == "INTEGRATION", q.stdout + q.stderr)
        return sha

    def get(self, tid):
        return self.call(f"/api/tasks/{tid}")[1]

    def status(self, tid):
        return self.get(tid).get("status")

    def history(self, tid):
        return " | ".join(e.get("text") or "" for e in self.get(tid).get("history", []))

    def batch(self, number):
        rows = self.call(f"/api/integration/project/{self.project}/batches")[1].get("batches", [])
        return next((b for b in rows if b["number"] == number), {})

    # --- the stage lines ---
    def hold(self, name):
        open(os.path.join(self.ctl, f"hold-{name}"), "w").close()

    def release(self, name):
        os.remove(os.path.join(self.ctl, f"hold-{name}"))

    def runs(self, name, what="start"):
        try:
            with open(os.path.join(self.ctl, "runs.log")) as f:
                return sum(1 for line in f if line.split() == [name, what])
        except OSError:
            return 0

    def until(self, fn, seconds=SLOW):
        return wait_for(fn, bool, seconds, 0.5, last=True)

    def settled(self):
        """A line that outlived its window would have seen its hold go by now: it looks every 0.2 s."""
        time.sleep(1)
        return True

    def ended(self, tid):
        """Dispatcher's walk saw its session's pane gone."""
        return "Session ended" in self.history(tid)

    def published(self, sha):
        return subprocess.run(["git", "-C", self.origin, "merge-base", "--is-ancestor", sha, "main"], capture_output=True).returncode == 0

    def main_now(self):
        return git(self.origin, "rev-parse", "main")


def check_outlives_restart(w):
    """1. AC-190: a Check outlives a restart of terminumd and is not started again."""
    w.hold("check-1")
    a = w.task("a")
    sha = w.queue(a, "a")
    check("AC-198 Verify runs before a batch, then the batch's Check", w.until(lambda: w.runs(f"verify-{a}", "end") and w.runs("check-1")))
    w.restart()
    check("AC-190 the Check runs on in its window through terminumd's restart", w.runs("check-1") == 1 and not w.runs("check-1", "end") and w.status(a) == "INTEGRATION")
    w.release("check-1")
    check("AC-190 terminumd found the Check's window: its status read, the batch published", w.until(lambda: w.status(a) == "DONE") and w.published(sha), w.history(a))
    check("AC-190 the Check ran once", w.runs("check-1") == 1)
    check("AC-191 After runs once the batch is published", w.until(lambda: w.runs("after-1", "end")))


def check_after_restarts(w):
    """2. AC-191: terminumd restarts beside a running After and the next batch's Check."""
    w.hold("after-2")
    w.hold("check-3")
    b = w.task("b")
    w.queue(b, "b")
    check("AC-191 batch 2 published, its After runs", w.until(lambda: w.runs("after-2") and w.status(b) == "DONE"))
    c = w.task("c")
    sha = w.queue(c, "c")
    check("AC-191 the next batch is checked while After runs", w.until(lambda: w.runs("check-3")))
    w.restart()
    w.release("after-2")
    two = w.until(lambda: (b2 := w.batch(2)).get("ended") and b2) or {}
    check("AC-191 after the restart, After's window is found and its status read: batch 2 ends green", two.get("result") == "green" and w.runs("after-2") == 1, two)
    w.release("check-3")
    check("AC-190 the Check beside it goes on: batch 3 published, checked once",
          w.until(lambda: w.status(c) == "DONE") and w.published(sha) and w.runs("check-3") == 1, w.history(c))
    w.until(lambda: w.batch(3).get("ended"))


def check_server_ends(w):
    """3. AC-187, AC-190, AC-191, AC-198: the tmux server ends with a Verify, a Check and an After running."""
    d, e, f = w.task("d"), w.task("e"), w.task("f")
    for name in ("after-4", "check-5", f"verify-{f}"):
        w.hold(name)
    w.queue(d, "d")
    check("setup: batch 4's After runs", w.until(lambda: w.runs("after-4")))
    e_sha = w.queue(e, "e")
    check("setup: batch 5's Check runs", w.until(lambda: w.runs("check-5")))
    w.queue(f, "f")
    check("setup: F's Verify runs", w.until(lambda: w.runs(f"verify-{f}")))
    end_tmux(SOCKET)   # a crash, kill-server or a reboot: every pane gone at once
    check("AC-198 a Verify lost with the server runs again from its commit", w.until(lambda: w.runs(f"verify-{f}") == 2))
    check("AC-190 a Check lost with the server runs again", w.until(lambda: w.runs("check-5") == 2))
    five = w.batch(5)
    check("AC-190 the Check run again is said, and not red",
          "Batch 5 is checked again: its window was lost." in w.history(five.get("task")) and five.get("result") == "checking" and five.get("rounds") == 1, five)
    lost = "After's outcome is not known for batch 4: its window was lost. After is not run again."
    four = w.until(lambda: lost in ((b4 := w.batch(4)).get("result") or "") and b4) or {}
    check("AC-191 an After lost with the server is not run again, its row says so", bool(four) and w.runs("after-4") == 1, w.batch(4))
    bug = four.get("result", "").rsplit("Check its work: ", 1)[-1].rstrip(".")
    shown = w.get(bug)
    check("AC-191 a P1 Bug task of the project, the human's, asks to check After's work",
          shown.get("title") == "Bug: After's outcome is not known for batch 4 of Billing" and shown.get("severity") == "P1" and shown.get("owner_kind") == "human", shown)
    check("AC-187 queued tasks stay queued when their sessions end with the server",
          w.until(lambda: w.ended(e) and w.ended(f), 90) and w.status(e) == "INTEGRATION" and w.status(f) == "INTEGRATION", f"{w.history(e)} || {w.history(f)}")
    check("AC-190 nothing is published but a green candidate", not w.published(e_sha))
    w.release("check-5")
    w.release(f"verify-{f}")
    check("AC-190 batch 5 published once its Check ran again green", w.until(lambda: w.status(e) == "DONE") and w.published(e_sha), w.history(e))
    check("AC-187 the task whose session ended lands as any", w.until(lambda: w.status(f) == "DONE"), w.history(f))
    check("AC-191 batch 4's After never ran again", w.runs("after-4") == 1)
    w.release("after-4")
    check("AC-190 the lines lost with the server ended with it: none runs on beside its run again",
          w.settled() and w.runs("check-5", "end") == 1 and w.runs(f"verify-{f}", "end") == 1 and w.runs("after-4", "end") == 0)


def check_publish_ends(w):
    """4. AC-191: the tmux server ends while the project's Publish runs, its push done."""
    w.settings(publish=f'git push -q origin "$TM_INTEGRATION_SHA:main" && {w.stage("publish", "$TM_INTEGRATION_BATCH")}')
    w.until(lambda: w.batch(6).get("ended"))
    before = w.main_now()
    w.hold("publish-7")
    g = w.task("g")
    sha = w.queue(g, "g")
    check("setup: batch 7's Publish pushed and runs on", w.until(lambda: w.runs("publish-7")) and w.published(sha))
    end_tmux(SOCKET)
    check("AC-191 a Publish lost with the server runs again", w.until(lambda: w.runs("publish-7") == 2))
    w.release("publish-7")
    check("AC-191 the lost Publish ended with the server", w.until(lambda: w.runs("publish-7", "end")) and w.settled() and w.runs("publish-7", "end") == 1)
    check("AC-191 the batch is published once", w.until(lambda: w.status(g) == "DONE") and w.history(g).count("Integrated into") == 1, w.history(g))
    check("AC-191 never forced: the target only moved forward", w.published(before))


def main():
    work = os.path.realpath(tempfile.mkdtemp(prefix="tmi-", dir=short_tmp()))
    w = World(work)
    try:
        w.start()
        w.setup()
        for scenario in (check_outlives_restart, check_after_restarts, check_server_ends, check_publish_ends):
            started = time.monotonic()
            scenario(w)
            print(f"  {scenario.__name__}: {time.monotonic() - started:.0f} s")
    except Exception as e:   # a step that cannot go on fails the run with terminumd's last words
        check(f"the scenarios run to their end: {e}", False, w.t.log())
    finally:
        w.t.cleanup()
        shutil.rmtree(work, ignore_errors=True)
    summary()


if __name__ == "__main__":
    main()
