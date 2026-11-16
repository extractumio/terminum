# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-169/AC-179: a manager promotes P0 behind P2/P2 and keeps the chosen agent's conversation.
Called by e2e_roles.py inside its own real terminumd/tmux, through the manager's actual pane."""
import json
import os
import sqlite3

from e2e_ws import BIN, check, wait_for


def queue_promotion(t, call, typed, tool, boss, plain, project, fake):
    agent = call("/api/task-agents", "POST", {"project": project, "name": "Queue", "kind": "claude",
                 "command": f"python3 -I {fake} {BIN} --kind claude", "at_once": 1})[1]["id"]

    def task(title, severity="P2"):
        return call("/api/tasks", "POST", {"title": title, "project": project, "severity": severity})[1]["id"]

    def get(id):
        return call(f"/api/tasks/{id}")[1]

    def queued():
        return [row["id"] for row in call(f"/api/tasks?project={project}&limit=1&counts=1")[1]["slots"].get("queue", {}).get(agent, [])]

    def latest(id):
        with sqlite3.connect(f"file:{os.path.join(t.app, 'kanban.sqlite')}?mode=ro", uri=True) as db:
            return db.execute("SELECT r.id, r.agent_id, r.agent_session FROM runs r JOIN run_tasks m ON m.run = r.id "
                              "JOIN tasks t ON t.id = m.task WHERE t.slug = ? ORDER BY m.rowid DESC LIMIT 1", [id]).fetchone()

    def kept(id):
        with sqlite3.connect(f"file:{os.path.join(t.app, 'kanban.sqlite')}?mode=ro", uri=True) as db:
            return db.execute("SELECT w.agent_id, w.project, w.body, w.at FROM run_waits w JOIN tasks t ON t.id = w.task WHERE t.slug = ?", [id]).fetchone()

    urgent = task("urgent QUESTIONS", "P0")
    out, rc = typed(boss, tool("run_task", id=urgent, agent="Queue"), 40)
    asked = wait_for(lambda: get(urgent), lambda v: v["status"] == "WAIT FOR HUMAN" and v.get("session", {}).get("closed"), 30, 0.3, last=True)
    check("AC-169 setup: a P0 asks with a resumable conversation", rc == "0" and asked["status"] == "WAIT FOR HUMAN" and latest(urgent)[2], out)
    saved = latest(urgent)
    call(f"/api/tasks/{urgent}", "PATCH", {"title": "urgent WORKS"})
    holder = task("queue holder WORKS")
    typed(boss, tool("run_task", id=holder, agent="Queue"), 40)
    check("AC-169 setup: another run occupies the agent's only slot", wait_for(lambda: get(holder)["status"], lambda v: v == "IN PROGRESS", 20, 0.3, last=True) == "IN PROGRESS")
    one, two = task("older one WORKS"), task("older two WORKS")
    for id in [one, two, urgent]:
        out, rc = typed(boss, tool("run_task", id=id, agent="Queue", queue="yes", comment="WORKS: use v2, keeping this answer."), 40)
        check(f"AC-169 {id} waits for the same agent", rc == "0" and "waits for a slot" in out, out)
    check("AC-169 P0 joins after P2/P2 until deliberately promoted", queued() == [one, two, urgent], queued())
    before = kept(urgent)
    out, rc = typed(plain, tool("promote_queued_task", id=urgent))
    check("AC-174 a worker cannot promote the waiting run", rc == "1" and 'No tool is named "promote_queued_task"' in out and queued() == [one, two, urgent], out)
    out, rc = typed(boss, tool("promote_queued_task", id=urgent))
    check("AC-169 the manager moves P0 to the front, keeping the P2 order", rc == "0" and "is now at the front" in out and queued() == [urgent, one, two], out)
    check("AC-169 promotion preserves the saved press, answer, agent and conversation", kept(urgent) == before and latest(urgent) == saved and get(holder)["status"] == "IN PROGRESS")
    check("AC-169 the card projection follows the manager's order", get(urgent).get("wait_position") == 1 and get(one).get("wait_position") == 2 and get(two).get("wait_position") == 3)
    out, rc = typed(boss, tool("list_runs"))
    waits = json.loads(out.split("\n", 1)[1])["waiting_for_a_slot"]
    check("AC-179 list_runs reports persisted order rather than arrival timestamps", rc == "0" and [w["task"] for w in waits if w["waits_for"] == "Queue"] == [urgent, one, two], waits)
    history = get(urgent)["history"]
    check("AC-169 history identifies the manager's deliberate promotion", any("Shell · manager moved it to the front of Queue's queue." in h.get("text", "") for h in history), history)
    out, rc = typed(boss, tool("promote_queued_task", id=urgent))
    check("AC-169 promoting first again is an idempotent success", rc == "0" and "is already" in out and get(urgent)["history"] == history, out)
    typed(boss, tool("close_session", session=get(holder)["session"]["key"]))
    started = wait_for(lambda: get(urgent), lambda v: v["status"] == "IN PROGRESS" and not v.get("waits"), 30, 0.3, last=True)
    resumed = latest(urgent)
    check("AC-169 the next eligible vacancy resumes P0 on the same agent and conversation", started["status"] == "IN PROGRESS" and resumed[0] != saved[0] and resumed[1:] == saved[1:] and queued() == [one, two], (resumed, saved, queued()))
    out, rc = typed(boss, tool("promote_queued_task", id=urgent))
    check("AC-169 an already-started promotion is refused without restarting it", rc == "1" and "no waiting run of its own" in out and latest(urgent) == resumed, out)
