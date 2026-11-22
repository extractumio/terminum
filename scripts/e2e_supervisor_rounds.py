#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-233–235: private Dispatcher and fake agent, exercising the installed CLI's MCP wire protocol."""
from contextlib import closing
import json
from pathlib import Path
import shlex
import sqlite3
import time

from e2e_remote import api
from e2e_ws import BIN, Terminumd, check, summary, tmux_on, unix_request, wait_for
from testrun import free_port, own

FAKE = Path(__file__).resolve().with_name("fake_agent.py")
RULE = "flow.failed_no_reason"
PORT, SOCKET = free_port(), own("tm-test-sround")


def main():
    t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1", "TM_TEST_CLOCK_SCALE": "60"})
    db_path = Path(t.app, "kanban.sqlite")
    prompts = Path(t.root, "round-prompts")
    log = Path(t.root, "round-tools")
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"}
    secret = ""
    comment = "Fixture comment used as data. " * 150

    def db(statement, bindings=()):
        with closing(sqlite3.connect(db_path)) as conn, conn:
            return conn.execute(statement, bindings).fetchall()

    def request(path, method="GET", body=None, signed=True):
        return api(t, secret if signed else "", path, method=method,
                   body=json.dumps(body).encode() if body is not None else None, headers=headers)

    def call(path, method="GET", body=None):
        status, reply = request(path, method, body)
        assert status == 200, (path, status, reply)
        return reply

    def logged():
        return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

    def taken():
        return prompts.read_text().splitlines() if prompts.exists() else []

    def configure(path, project, mode, room=True, foreign=""):
        agent = call("/api/task-agents", "POST", {
            "project": project, "name": mode, "kind": "claude", "at_once": 1,
            "command": shlex.join(["python3", "-I", str(FAKE), BIN, "--prompts", str(prompts),
                                  "--round-log", str(log), "--round-mode", mode, "--foreign-evidence", foreign]),
        })["id"]
        saved = call(path + "/settings")
        pack = saved["settings"]
        pack["level"] = "observer"
        pack["tiers"] = dict(medium=agent, small=agent, strong=agent)
        if not room:
            pack["budgets"]["daily_token_ceiling"] = 0
        call(path + "/settings", "PUT", {"revision": saved["revision"], "settings": pack})

    def seeded(path, project):
        tasks = []
        for _ in range(4):
            task = call("/api/tasks", "POST", {"title": "Round fixture", "text": "Fixture task body.", "project": project})["id"]
            call(f"/api/tasks/{task}", "PATCH", {"status": "FAILED"})
            call(f"/api/tasks/{task}/comments", "POST", {"text": comment})
            tasks.append(task)
        call(path, "PUT", {"enabled": True})
        query = "SELECT fingerprint FROM supervisor_signals WHERE project = ? AND detector = ?"
        return wait_for(lambda: db(query, (project, RULE)), bool, 20, every=0.2)[0][0]

    def ended(project, count):
        query = "SELECT id, deadline, ended_at, failure, refused, finding FROM supervisor_rounds WHERE project = ? ORDER BY rowid"
        return wait_for(lambda: db(query, [project]),
                        lambda rows: len(rows) == count and all(row[2] for row in rows), 30, every=0.2)

    def raise_level(key, confidence):
        # The collector supplied the signal, named query, measure and IDs. A drill raises only its stakes.
        db("UPDATE supervisor_signals SET scores = json_set(scores, '$.blast', 2, '$.confidence', ?) WHERE fingerprint = ?",
           (confidence, key))

    try:
        t.start()
        secret = t.signin()
        projects = [call("/api/task-projects", "POST", {"name": name, "dir": t.root})["id"]
                    for name in ("Rounds", "Deadlines")]
        project, expiry_project = projects
        path, expiry_path = [f"/api/task-projects/{pid}/supervisor" for pid in projects]
        foreign = call("/api/tasks", "POST", {"title": "Other project evidence", "project": expiry_project})["id"]
        foreign_key = db("SELECT id FROM tasks WHERE slug = ?", [foreign])[0][0]
        configure(path, project, "finding", foreign=foreign_key)
        key = seeded(path, project)
        raise_level(key, confidence=0.55)
        first = ended(project, 1)[0]
        case, _, _, failure, refused, finding = first
        retrieved = call(f"{path}/rounds/{case}")
        typed = f"Supervisor case {case}. Call get_case_file with its ID and report through the Supervisor tools."
        check("AC-234 only the fixed prompt with a case ID was typed", taken() == [typed], taken())
        check("AC-234 the agent starts with its own scoped role before its first hook",
              logged()[0]["rc"] == 0 and "call as a supervisor" in logged()[0]["probe"]
              and "get_case_file" in logged()[0]["probe"] and "send_to_session" not in logged()[0]["probe"], logged()[0])
        reads = [item for item in logged() if item.get("tool") == "get_case_file"]
        check("AC-234 case access is exact, durable and bounded",
              len(reads) == 2 and reads[0]["isError"] and not reads[1]["isError"]
              and retrieved["case"] == json.loads(reads[1]["content"][0]["text"])
              and len(retrieved["case"]["task_data"][0]["comments_data"][0]) == 1000
              and comment not in taken()[0])
        check("AC-233 low confidence uses the verification path and a 15-minute cap",
              retrieved["case"]["kind"] == "verification" and retrieved["case"]["budget_seconds"] == 900
              and retrieved["case"]["verification_hold"])
        attempts = [item for item in logged() if item.get("tool") == "record_finding"]
        check("AC-235 a wrong number, a missing row and a foreign row signal their refusal",
              len(attempts) == 4 and attempts[0]["isError"] and "returned 4" in attempts[0]["content"][0]["text"]
              and attempts[1]["isError"] and "does not resolve" in attempts[1]["content"][0]["text"]
              and attempts[2]["isError"] and "does not resolve" in attempts[2]["content"][0]["text"]
              and refused == 3 and db("SELECT count FROM supervisor_signals WHERE subject_id = ? AND detector = 'self.round_failed'", [case]) == [(3,)],
              attempts)
        check("AC-235 the correct named measurement and evidence are kept",
              not failure and json.loads(finding)["measurements"][0]["value"] == 4 and not attempts[-1]["isError"], first)
        db("UPDATE supervisor_signals SET count = count + 1 WHERE fingerprint = ?", [key])
        time.sleep(3)
        check("AC-233 an unchanged repeat has exactly one round and one prompt",
              db("SELECT count(*) FROM supervisor_rounds WHERE fingerprint = ?", [key]) == [(1,)] and taken() == [typed],
              call(f"{path}/screen")["signals"][0])
        check("AC-234 an archived case is signed-in and project scoped",
              request(f"{path}/rounds/{case}", signed=False)[0] == 401 and request(f"{expiry_path}/rounds/{case}")[0] != 200)

        t.stop()
        t.start(keep_tmux=True)
        secret = t.signin()
        check("AC-234 a daemon restart preserves the case and does not type again",
              call(f"{path}/rounds/{case}") == retrieved and taken() == [typed])

        configure(expiry_path, expiry_project, "deadline", room=False)
        key = seeded(expiry_path, expiry_project)
        route = f"{expiry_path}/signals/{key}/investigate"
        check("AC-233 the signed owner action enforces the budget and is absent from the agent socket",
              request(route, "POST", {}, signed=False)[0] == 401 and unix_request(t.sock, "POST", route, {})[0] == 404
              and "budget" in request(route, "POST", {})[1]["error"].lower())
        saved = call(expiry_path + "/settings")
        saved["settings"]["budgets"]["daily_token_ceiling"] = None
        call(expiry_path + "/settings", "PUT", {"revision": saved["revision"], "settings": saved["settings"]})
        call(route, "POST", {})
        call(route, "POST", {})
        wait_for(taken, lambda lines: len(lines) == 2, 12, every=0.2)
        active = db("SELECT d.pane_key FROM supervisor_rounds r JOIN runs d ON d.id = r.run WHERE r.project = ?", [expiry_project])
        pane = active[0][0].rsplit(":", 1)[1]
        check("AC-233 the signed press starts one round and a repeat joins the active case",
              len(active) == 1 and "already open" in call(route, "POST", {})["message"])
        late_case, deadline, end, failure, _, finding = ended(expiry_project, 1)[0]
        panes = tmux_on(SOCKET, "list-panes", "-a", "-F", "#{pane_id}").splitlines()
        check("AC-234 Dispatcher closes an expired round and frees its kept slot",
              end >= deadline and failure and "deadline" in failure and finding is None and pane not in panes,
              (late_case, deadline, end, failure, panes))
        at = int(time.time())
        incident = db("INSERT INTO supervisor_incidents (project, subject_kind, subject_id, opened_at, last_firing_at) "
                      "VALUES (?, 'project', ?, ?, ?) RETURNING id", (expiry_project, expiry_project, at, at))[0][0]
        db("INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, level, value, threshold) "
           "VALUES (?, ?, 'signal', 'system', ?, 2, 4, 1)", (incident, at, key))
        call(f"/api/supervisor/incidents/{incident}/investigate", "POST", {})
        check("AC-233 the incident press requests its signal after the kept slot is freed",
              bool(wait_for(lambda: db("SELECT run FROM supervisor_rounds WHERE project = ?", [expiry_project]), lambda r: len(r) == 2, 10, every=0.2)))
        wait_for(taken, lambda lines: len(lines) == 3, 12, every=0.2)
        call(expiry_path, "PUT", {"enabled": False})
        check("AC-216 disabling the project ends its active investigation",
              "disabled" in ended(expiry_project, 2)[1][3].lower())
    except Exception:
        print(t.log())
        raise
    finally:
        t.cleanup()


if __name__ == "__main__":
    main()
    summary()
