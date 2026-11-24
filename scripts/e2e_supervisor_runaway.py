#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-213, AC-245: the cost outlier's runaway raise through the real collector, Dispatcher and owner keys in a
private terminumd and tmux. A Bug task's marked run of the fake agent (scripts/fake_agent.py) spends far past
twenty earlier Bug tasks: Auto-Dispatcher started it, and the detector raises it to level 3; while replay has not qualified holds the stop is a
shadow and the run works on; once holds are eligible code stops the run without a round; the owner's RUN of the
unchanged work starts again and code does not stop it again.

    python3 scripts/e2e_supervisor_runaway.py      (needs terminumd built: cd terminumd && cargo build)
"""
from contextlib import closing
import json
import os
from pathlib import Path
import sqlite3
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_remote import api  # noqa: E402
from e2e_ws import BIN, Terminumd, check, summary, wait_for  # noqa: E402
from testrun import free_port, own  # noqa: E402

DAY = 86400
RULE = "flow.cost_outlier"
FAKE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fake_agent.py")
# claude-opus-5-5's output price is $20 a million tokens.
DOLLAR = 50_000


def main():
    port = free_port()
    server = Terminumd(port, own("tm-test-runaway"), env={"TM_TEST_HOOKS": "1"})
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{port}"}

    def db(sql, args=()):
        with closing(sqlite3.connect(Path(server.app, "kanban.sqlite"))) as conn, conn:
            return conn.execute(sql, args).fetchall()

    def call(path, method="GET", body=None):
        status, reply = api(server, credential, path, method=method, headers=headers,
                            body=None if body is None else json.dumps(body).encode())
        assert status == 200, (path, status, reply)
        return reply

    def spend(run, at, dollars):
        db("INSERT INTO usage (run, agent_id, at, input, output, cache_write, cache_read, model) "
           "SELECT id, agent_id, ?, 0, ?, 0, 0, 'claude-opus-5-5' FROM runs WHERE id = ?", (at, int(dollars * DOLLAR), run))

    def run_of(slug):
        return db("SELECT r.id, r.state, r.pane_key FROM runs r JOIN run_tasks m ON m.run = r.id JOIN tasks t ON t.id = m.task "
                  "WHERE t.slug = ? ORDER BY r.rowid DESC LIMIT 1", [slug])

    def stops(slug):
        return db("SELECT h.id, h.shadow, h.released_at FROM supervisor_holds h JOIN tasks t ON t.id = h.target "
                  "WHERE h.kind = 'stop_run' AND t.slug = ? ORDER BY h.id", [slug])

    def signal(slug):
        rows = db("SELECT json_extract(s.scores, '$.impact'), json_extract(s.scores, '$.raised.run') FROM supervisor_signals s "
                  "JOIN tasks t ON t.id = s.subject_id WHERE s.detector = ? AND s.metric = 'task_dollars' AND t.slug = ?",
                  (RULE, slug))
        return rows[0] if rows else None

    try:
        server.start()
        credential = server.signin()
        project = call("/api/task-projects", "POST", {"name": "Runaway fixture", "dir": server.root})["id"]
        path = f"/api/task-projects/{project}/supervisor"
        agent = call("/api/task-agents", "POST", {"project": project, "name": "Fake", "kind": "claude",
                                                  "command": f"python3 -I {FAKE} {BIN} --kind claude", "at_once": 2})["id"]
        at = int(time.time())
        for n in range(20):
            slug = call("/api/tasks", "POST", {"title": f"Earlier bug {n}", "project": project, "type": "Bug"})["id"]
            [[key]] = db("SELECT id FROM tasks WHERE slug = ?", [slug])
            old, run = at - 36 * DAY, f"earlier-{n}"
            db("UPDATE tasks SET created_at = ?, status = 'DONE' WHERE id = ?", (old, key))
            db("UPDATE events SET at = ? WHERE task = ?", (old, key))
            db("INSERT INTO runs (id, task, project, agent_id, kind, command, folder, state, started_by, started_how) "
               "VALUES (?, ?, ?, ?, 'claude', '', '', 'closed', 'human', 'run')", (run, key, project, agent))
            db("INSERT INTO run_tasks (run, task) VALUES (?, ?)", (run, key))
            db("INSERT INTO inputs (at, ended, sender, component, kind, text, bytes, pane, run, version) "
               "VALUES (?, ?, 'dispatcher', 'dispatcher.start', 'launch', '', 0, '', ?, '')", (old * 1000, old * 1000, run))
            spend(run, old + 60, 1 + n / 10)

        # Auto-Dispatcher starts it: a human's RUN would hold the Supervisor off the task for 24 hours (AC-239).
        task = call("/api/tasks", "POST", {"title": "Runaway fixture WORKS", "project": project, "type": "Bug"})["id"]
        auto = f"/api/task-projects/{project}/auto"
        call(auto, "PUT", {"on": True, "mode": "order", "default": agent, "severities": ["P2"], "wait_minutes": 0})
        first = wait_for(lambda: run_of(task), lambda rows: rows and rows[0][1] == "working" and rows[0][2], 60, 0.3, last=True)
        call(auto, "PUT", {"on": False})
        check("setup: Auto-Dispatcher's marked run of the fake agent works on the task", bool(first and first[0][1] == "working"), first)
        original = first[0][0]
        spend(original, int(time.time()), 40)

        call(path, "PUT", {"enabled": True})
        shadow = wait_for(lambda: stops(task), bool, 40, 0.5, last=True)
        check("AC-213 the real detector raises the fresh $40 run to level 3, naming that run",
              signal(task) == (3, original), signal(task))
        check("AC-221 AC-245 before replay qualifies holds, code records a shadow stop and the run works on",
              shadow and shadow[0][1] == 1 and run_of(task)[0][:2] == (original, "working")
              and call(f"/api/tasks/{task}")["status"] == "IN PROGRESS"
              and not db("SELECT 1 FROM supervisor_rounds"), (shadow, run_of(task)))

        # The detector's holds become eligible as after a week attached; a fresh report makes the collector look again.
        scan = f'$.detectors."{RULE}"'
        db("UPDATE project_supervisor SET attachment = json_set(attachment, '$.attached_at', ?, ?, json('true')) "
           "WHERE project = ?", (at - 8 * DAY, scan + ".can_run", project))
        spend(original, int(time.time()), 1)
        live = wait_for(lambda: [h for h in stops(task) if h[1] == 0], bool, 40, 0.5, last=True)
        held = call(path + "/screen")["holds"]
        check("AC-245 code stops the named run without a round: the task FAILED, its pane kept, RUN its undo",
              live and call(f"/api/tasks/{task}")["status"] == "FAILED" and run_of(task)[0][0] == original
              and run_of(task)[0][1] in ("left", "closed") and any(h["id"] == live[0][0] and h["undo"] == "RUN" for h in held)
              and not db("SELECT 1 FROM supervisor_rounds"), (live, run_of(task), held))

        call(f"/api/tasks/{task}/run", "POST", {"agent": agent})
        resumed = wait_for(lambda: run_of(task), lambda rows: rows and rows[0][0] != original
                           and rows[0][1] not in ("starting", "asking", "typed"), 60, 0.3, last=True)
        released = wait_for(lambda: stops(task), lambda rows: all(r[2] for r in rows if r[1] == 0), 20, 0.3, last=True)
        # Two more collector passes over the unchanged work, each made due by a fresh report of the resumed run.
        for _ in range(2):
            [[before]] = db("SELECT last_evaluation FROM project_supervisor WHERE project = ?", [project])
            spend(resumed[0][0], int(time.time()), 0.5)
            wait_for(lambda: db("SELECT last_evaluation FROM project_supervisor WHERE project = ?", [project]),
                     lambda rows: rows[0][0] and rows[0][0] > before, 30, 0.3)
        check("AC-245 the owner's RUN releases the stop and the unchanged work is not stopped again",
              resumed and resumed[0][0] != original and released
              and len([h for h in stops(task) if h[1] == 0]) == 1
              and call(f"/api/tasks/{task}")["status"] != "FAILED"
              and (signal(task) or (0,))[0] < 3, (resumed, stops(task), signal(task)))
    finally:
        server.cleanup()


if __name__ == "__main__":
    main()
    summary()
