#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-212, AC-224: a red integration target opens a critical incident in a private terminumd, and clears it."""
import json
import os
import sqlite3
from contextlib import closing

from e2e_remote import api
from e2e_ws import Terminumd, check, summary, wait_for
from testrun import free_port, own

PORT, SOCKET = free_port(), own("tm-test-supervisor-target")
NEVER = "Leave the target red."


def main():
    t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1"})  # the live tests place cards by a status change
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"}
    path = os.path.join(t.app, "kanban.sqlite")

    def read(sql, args=()):
        with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as db:
            return db.execute(sql, args).fetchall()

    def write(sql, args=()):
        with closing(sqlite3.connect(path)) as db, db:
            db.execute(sql, args)

    try:
        t.start()
        session = t.signin()

        def call(url, method="GET", data=None):
            body = json.dumps(data).encode() if data is not None else None
            status, result = api(t, session, url, method=method, body=body, headers=headers)
            assert status == 200, (url, status, result)
            return result

        project = call("/api/task-projects", "POST", {"name": "API", "dir": t.root})["id"]
        path_of = f"/api/task-projects/{project}/supervisor"
        call(path_of + "/settings", "PUT", {"revision": 0, "settings": {"charter": {
            "never": [NEVER], "never_checks": {"flow.main_red": NEVER}}}})
        # Integration's own settings window needs its stage lines; the detector reads only the switch and the target.
        write("""INSERT INTO project_integration (project, is_on, target, check_line, publish_line, after_line, batch_most,
                 check_minutes, agent_budget_minutes, red_rounds_most, prompt) VALUES (?, 1, 'origin/main', '', '', '', 4, 30, 60, 3, '')""",
              (project,))
        call(path_of, "PUT", {"enabled": True})
        wait_for(lambda: call(path_of), lambda s: s["last_ok"] is not None, 30)

        def incidents():
            return read("""SELECT i.id, i.closed_at, max(e.level) FROM supervisor_incidents i
                JOIN supervisor_incident_events e ON e.incident = i.id AND e.detector = 'flow.main_red'
                WHERE i.project = ? GROUP BY i.id""", (project,))

        check("AC-224 a green target opens no incident", not incidents())
        bug = call("/api/tasks", "POST", {"title": "Bug: origin/main is red", "project": project})
        key = read("SELECT id FROM tasks WHERE slug = ?", (bug["id"],))[0][0]
        # Dispatcher marks its red-target Bug task in the same write that makes it (terminum-integration.md AC-198).
        write("UPDATE project_integration SET red_bug = ? WHERE project = ?", (key, project))
        call(f"/api/tasks/{bug['id']}/comments", "POST", {"text": "The target's Verify steps fail."})
        opened = wait_for(incidents, lambda rows: len(rows) == 1, 70, every=0.5)
        check("AC-212 AC-224 a red target tied to a never line opens one level 3 incident",
              opened[0][1] is None and opened[0][2] == 3, str(opened))
        signal = read("SELECT subject_kind, subject_id, state FROM supervisor_signals WHERE project = ? AND detector = 'flow.main_red'",
                      (project,))
        check("AC-212 its signal names the target", signal == [("target", "origin/main", "new")], str(signal))

        call(f"/api/tasks/{bug['id']}", "PATCH", {"status": "DONE"})
        cleared = wait_for(incidents, lambda rows: rows and rows[0][1] is not None, 70, every=0.5)
        check("AC-224 the red-target Bug task's end gives the all-clear on the same incident",
              len(cleared) == 1 and cleared[0][0] == opened[0][0], str(cleared))
    finally:
        t.cleanup()


if __name__ == "__main__":
    main()
    summary()
