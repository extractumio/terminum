#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-246 and AC-247 through the real collector and signed-in owner routes in private tmux."""
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import time

from e2e_remote import api
from e2e_ws import Terminumd, check, summary, unix_request, wait_for
from testrun import free_port, own

DAY = 86400
RULE = "flow.failed_repeat"


def main():
    port = free_port()
    server = Terminumd(port, own("tm-test-outcomes"))
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{port}"}

    def db(sql, args=()):
        with closing(sqlite3.connect(Path(server.app, "kanban.sqlite"))) as conn, conn:
            return conn.execute(sql, args).fetchall()

    def request(path, method="GET", body=None, signed=True):
        return api(server, credential if signed else "", path, method=method,
                   headers=headers, body=None if body is None else json.dumps(body).encode())

    def call(path, method="GET", body=None):
        status, reply = request(path, method, body)
        assert status == 200, (path, status, reply)
        return reply

    try:
        server.start()
        credential = server.signin()
        project = call("/api/task-projects", "POST", {"name": "Outcome fixture"})["id"]
        path = f"/api/task-projects/{project}/supervisor"
        task = call("/api/tasks", "POST", {"title": "Replay history fixture", "project": project})["id"]
        at = int(time.time())
        db("UPDATE tasks SET created_at = ? WHERE slug = ?", (at - 56 * DAY, task))
        db("UPDATE events SET at = ? WHERE task = (SELECT id FROM tasks WHERE slug = ?) AND kind = 'made'",
           (at - 56 * DAY, task))

        incident = db("INSERT INTO supervisor_incidents (project,subject_kind,subject_id,opened_at,last_firing_at) "
                      "VALUES (?,'project','fixture',?,?) RETURNING id", (project, at, at))[0][0]
        event = db("INSERT INTO supervisor_incident_events (incident,at,kind,actor,fingerprint,detector,metric,level,value,threshold) "
                   "VALUES (?,?,'signal','system','trial',?,'failed_runs',3,5,2) RETURNING id", (incident, at, RULE))[0][0]
        db("INSERT INTO supervisor_signals (fingerprint,project,detector,subject_kind,subject_id,metric,evidence,measure,scores,"
           "first_seen,last_seen,count,state) VALUES ('trial',?,?,'project','fixture','failed_runs','[]',?,?,?, ?,1,'raised')",
           (project, RULE, json.dumps(dict(value=5, baseline=2, threshold=2, window_start=at-30, window_end=at)),
            json.dumps(dict(impact=3, blast=0, reversibility=0, confidence=1)), at, at))
        alert = db("INSERT INTO supervisor_actions (project,detector,response,kind,source,fingerprint,target,at) "
                   "VALUES (?,?,'R5','alert',?,'trial',?,?) RETURNING id", (project, RULE, str(event), str(incident), at))[0][0]
        fresh = db("INSERT INTO supervisor_actions (project,detector,response,kind,source,fingerprint,target,at) "
                   "VALUES (?,?,'R4','nudge','fixture-comment','no-evidence','fixture',?) RETURNING id", (project, RULE, at - 29 * DAY))[0][0]

        item = f"/api/supervisor/incidents/{incident}"
        wrong = {"action": "this_is_wrong", "alert": alert}
        check("AC-247 the unsigned page and agent socket cannot mark a verdict",
              request(item, "POST", wrong, signed=False)[0] == 401
              and unix_request(server.sock, "POST", item, wrong)[0] == 404)
        check("AC-247 the signed-in owner marks exactly one retained alert wrong",
              call(item, "POST", wrong)["alerts"][0]["wrong"]
              and call(item, "POST", wrong)["alerts"][0]["wrong"]
              and len([e for e in call(item)["rows"] if e["kind"] == "alert_wrong"]) == 1)
        [breaker] = call(path + "/screen")["breakers"]
        check("AC-247 one wrong alert opens only that detector and response",
              breaker["detector"] == RULE and breaker["response"] == "R5")
        review_path = f"{path}/breakers/{breaker['id']}/review"
        restore = f"{path}/breakers/{breaker['id']}/restore"
        check("AC-247 the owner cannot restore before a rule change and replay",
              request(review_path, "POST", {})[0] == 400 and request(restore, "POST", {"proof": "stale"})[0] == 400)

        settings = {"detectors": {RULE: {"threshold": {"absolute": 3}}}}
        call(path + "/settings", "PUT", {"revision": 0, "settings": settings})
        preview = call(review_path, "POST", {})
        check("AC-247 a real replay describes the old and new rule against the same history",
              preview["review"]["before"]["threshold"]["absolute"] == 2
              and preview["review"]["after"]["threshold"]["absolute"] == 3
              and preview["review"]["first"] <= at - 55 * DAY)
        proof = {"proof": preview["proof"]}
        check("AC-247 the agent socket and unsigned page cannot use the owner's key",
              unix_request(server.sock, "POST", restore, proof)[0] == 404
              and request(restore, "POST", proof, signed=False)[0] == 401
              and len(call(path + "/screen")["breakers"]) == 1)
        call(path + "/settings", "PUT", {"revision": 1, "settings": settings})
        check("AC-247 the owner key is invalid after a settings revision", request(restore, "POST", proof)[0] == 400)
        call(restore, "POST", {"proof": call(review_path, "POST", {})["proof"]})
        check("AC-247 only the reviewed owner key closes the breaker",
              not call(path + "/screen")["breakers"] and call(path + "/settings")["settings"]["level"] == "operator")

        call(path, "PUT", {"enabled": True})
        results = wait_for(lambda: db("SELECT days,verdict FROM supervisor_outcomes WHERE action = ? ORDER BY days", (fresh,)),
                           lambda rows: len(rows) == 3, 25, every=.2)
        screen = call(path + "/screen")
        check("AC-246 collector records each elapsed horizon and signals missing proof",
              results == [(1, "undecided"), (7, "undecided"), (28, "undecided")]
              and any(s["detector"] == "self.unjudged" for s in screen["signals"])
              and next(r for r in screen["outcomes"] if r["detector"] == RULE and r["response"] == "R4")["undecided"] == 1)
    finally:
        server.cleanup()


if __name__ == "__main__":
    main()
    summary()
