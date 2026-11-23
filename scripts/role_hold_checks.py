# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-221/245: run the real hold and inverse keys on the role drill's private terminumd."""
import json
import time

from e2e_ws import check, wait_for


def ac245_hold_undo_and_switch_off(call, db, wire, tmux, project, task, pane):
    supervisor = f"/api/task-projects/{project}/supervisor"
    integration = f"/api/integration/project/{project}/pause"
    auto = f"/api/task-projects/{project}/auto"
    at = int(time.time())
    rule = "drift.diff_outlier"

    def okay(path, method="GET", body=None):
        code, data = call(path, method, body)
        assert code == 200, (path, code, data)
        return data

    def signal(id, level, subject=None):
        measure = dict(value=4, baseline=2, threshold=3, window_start=at-30, window_end=at)
        scores = dict(impact=level, blast=0, reversibility=0, confidence=1.0)
        db("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric, "
           "evidence, measure, scores, first_seen, last_seen, count, state) "
           "VALUES (?, ?, ?, ?, ?, 'hold_fixture', '[]', ?, ?, ?, ?, 1, 'raised')",
           (id, project, rule, "task" if subject else "project", subject or project,
            json.dumps(measure), json.dumps(scores), at, at))

    def held(kind, signal="hold-wide", task=None):
        args = dict(kind=kind, signal=signal, reason="The measured threshold was exceeded.")
        if task:
            args["task"] = task
        return wire("hold", args)

    def applied(kind, signal="hold-wide", task=None):
        reply = held(kind, signal, task)
        assert not reply["error"], reply
        return json.loads(reply["text"])

    def active():
        return okay(supervisor + "/screen")["holds"]

    def state():
        [row] = db("SELECT paused FROM project_integration WHERE project = ?", [project])
        return row[0]

    signal("hold-wide", 2)
    signal("hold-low", 1)
    signal("hold-run", 2, task)
    db("INSERT INTO project_integration (project, is_on, target, check_line, publish_line, after_line, "
       "batch_most, check_minutes, agent_budget_minutes, red_rounds_most, prompt, paused) "
       "VALUES (?, 1, 'main', '', '', '', 1, 1, 1, 1, '', 0)", [project])
    wait_for(lambda: db("SELECT last_ok IS NOT NULL AND settings_revision IS coalesce((SELECT revision FROM supervisor_settings"
                        " WHERE project = ?), 0) FROM project_supervisor WHERE project = ?", [project, project]),
             lambda rows: rows == [(1,)], 15)
    scan = f'$.detectors."{rule}"'
    args = (scan + ".mode", scan + ".can_run", scan + ".holds.live_at_attach", project)
    db("UPDATE project_supervisor SET attachment = json_set(attachment, '$.attached_at', ?, ?, 'live', ?, json('true'), ?, json('true')) "
       "WHERE project = ?", (at, *args))
    check("AC-245 an absent or low signal and an unsupported kind are refused",
          held("pause_integration", "absent")["error"] and held("pause_integration", "hold-low")["error"]
          and held("delete")["error"] and wire("stop_task", {"id": task})["error"] and state() == 0)
    shadow = applied("pause_integration")
    check("AC-221 AC-245 a shadow hold records the detector and response with no queue change or incident",
          shadow["shadow"] and state() == 0 and not active()
          and db("SELECT shadow, detector, incident FROM supervisor_holds WHERE id = ?", [shadow["id"]]) == [(1, rule, None)])
    db("UPDATE project_supervisor SET attachment = json_set(attachment, '$.attached_at', ?) WHERE project = ?",
       (at - 8*86400, project))

    stopped = wire("stop_task", {"id": task, "signal": "hold-run", "reason": "Fixture measured run cost."})
    assert not stopped["error"], stopped
    run_hold = json.loads(stopped["text"])["id"]
    [old] = db("SELECT id, pane_key FROM runs WHERE pane_key LIKE ? ORDER BY rowid DESC LIMIT 1",
               [f"%:{pane}"])
    panels = tmux("list-panes", "-a", "-F", "#{pane_id}").split()
    check("AC-245 the Operator's Give up fails the work with a reason and leaves its pane",
          pane in panels and okay(f"/api/tasks/{task}")["status"] == "FAILED"
          and any(e.get("text") == "Fixture measured run cost." for e in okay(f"/api/tasks/{task}")["history"])
          and any(h["id"] == run_hold and h["undo"] == "RUN" for h in active()))
    reply = wire("release_hold", {"id": run_hold})
    check("AC-245 the Operator releases a stop with the same RUN admission path",
          not reply["error"] and not active()
          and db("SELECT released_by IS NOT NULL FROM supervisor_holds WHERE id = ?", [run_hold]) == [(1,)], reply)
    check("AC-174 AC-245 restarting held work preserves the old pane's worker identity",
          db("SELECT state, pane_key FROM runs WHERE id = ?", [old[0]]) == [("closed", old[1])])
    wait_for(lambda: okay(f"/api/tasks/{task}")["status"], lambda status: status == "IN PROGRESS", 12)
    okay(f"/api/tasks/{task}/give-up", "POST", {})

    okay(auto, "PUT", {"on": True, "mode": "order", "severities": ["P0"], "wait_minutes": 1440})
    dispatch_hold = applied("pause_dispatch")["id"]
    check("AC-245 a dispatch hold uses the real switch and names its undo",
          db("SELECT auto_on FROM projects WHERE id = ?", [project]) == [(0,)]
          and any(h["id"] == dispatch_hold and h["undo"] == "Turn on Auto-Dispatcher" for h in active()))
    okay(auto, "PUT", {"on": True, "off_why": "Paused by the Supervisor."})
    check("AC-245 the board's Auto-Dispatcher switch releases and audits the hold",
          db("SELECT auto_on FROM projects WHERE id = ?", [project]) == [(1,)]
          and db("SELECT released_by FROM supervisor_holds WHERE id = ?", [dispatch_hold]) == [("human",)])
    okay(auto, "PUT", {"on": False})

    pause_hold = applied("pause_integration")["id"]
    [pause] = active()
    incident = f"/api/supervisor/incidents/{pause['incident']}"
    check("AC-245 a level 2 integration pause opens a project incident",
          state() == 1 and pause["id"] == pause_hold and pause["undo"] == "Resume integration"
          and any(i["id"] == pause["incident"] and i["level"] == 2
                  and i["holds"] == [{"id": pause_hold, "label": pause["label"]}]
                  for i in okay("/api/supervisor/incidents")))
    check("AC-245 an incident refuses another hold's ID",
          call(incident, "POST", {"action": "release_hold", "hold": dispatch_hold})[0] == 400)
    okay(incident, "POST", {"action": "keep_hold", "hold": pause_hold})
    check("AC-245 Keep leaves the queue paused and writes the owner event",
          state() == 1 and any(e["kind"] == "hold_kept" and e["actor"] == "human" for e in okay(incident)["rows"]))

    pending = okay("/api/tasks", "POST", {"title": "Queued hold fixture", "project": project})["id"]
    [key] = db("SELECT id FROM tasks WHERE slug = ?", [pending])[0]
    db("UPDATE tasks SET status = 'INTEGRATION' WHERE id = ?", [key])
    sha, result = "a" * 40, "Retained integration result"
    db("INSERT INTO integration_entries (task, project, commit_sha, result, author, queued_at) "
       "VALUES (?, ?, ?, ?, 'dispatcher', ?)", (key, project, sha, result, at))
    signal("hold-entry", 2, key)
    removed = applied("take_out", "hold-entry", pending)["id"]
    check("AC-245 Take out withdraws one queued task and lists Queue for integration",
          okay(f"/api/tasks/{pending}")["status"] == "IN PROGRESS"
          and not db("SELECT 1 FROM integration_entries WHERE task = ?", [key])
          and any(h["id"] == removed and h["undo"] == "Queue for integration" for h in active()))
    okay(f"{supervisor}/holds/{removed}/release", "POST", {})
    check("AC-245 Queue for integration restores the original commit and result",
          db("SELECT commit_sha, result FROM integration_entries WHERE task = ?", [key]) == [(sha, result)]
          and okay(f"/api/tasks/{pending}")["status"] == "INTEGRATION")
    okay(f"/api/integration/task/{pending}/out", "POST", {})

    off = okay(supervisor, "PUT", {"enabled": False})
    check("AC-245 switching Supervisor off still lists its hold and leaves the queue paused",
          [h["id"] for h in off["holds"]] == [pause_hold] and state() == 1
          and [h["id"] for h in active()] == [pause_hold])
    okay(incident, "POST", {"action": "release_hold", "hold": pause_hold})
    check("AC-245 Release undoes the queue pause with Supervisor off and keeps both audit records",
          state() == 0 and not active() and db("SELECT actor, released_by, detector, level FROM supervisor_holds WHERE id = ?",
                                              [pause_hold]) == [(f"supervisor:{project}", "human", rule, 2)]
          and any(e["kind"] == "hold_released" for e in okay(incident)["rows"]))
    okay(supervisor, "PUT", {"enabled": True})
