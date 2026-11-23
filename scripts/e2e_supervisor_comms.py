#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-242–244: Routines, verdicts, the exception digest and the incident thread in private tmux."""
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
PORT, SOCKET = free_port(), own("tm-test-scomms")


def main():
    t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1", "TM_TEST_CLOCK_SCALE": "60"})
    prompts = Path(t.root, "comms-prompts")
    log = Path(t.root, "comms-log")
    barrier = Path(t.root, "thread-go")
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"}
    secret = ""

    def db(statement, bindings=()):
        with closing(sqlite3.connect(Path(t.app, "kanban.sqlite"))) as conn, conn:
            return conn.execute(statement, bindings).fetchall()

    def request(path, method="GET", body=None, signed=True):
        return api(t, secret if signed else "", path, method=method,
                   body=json.dumps(body).encode() if body is not None else None, headers=headers)

    def call(path, method="GET", body=None):
        status, reply = request(path, method, body)
        assert status == 200, (path, status, reply)
        return reply

    def create_project(title):
        project = call("/api/task-projects", "POST", {"name": title, "dir": t.root})["id"]
        return project, f"/api/task-projects/{project}/supervisor"

    def make_agent(project, title, extra=()):
        command = shlex.join(["python3", "-I", str(FAKE), BIN, "--prompts", str(prompts), "--round-log", str(log), *extra])
        return call("/api/task-agents", "POST",
                    {"project": project, "name": title, "kind": "claude", "at_once": 1, "command": command})["id"]

    def enable(path, project, agent=None):
        settings = {"level": "advisor", "budgets": {"judgment_floor_tokens": 400000, "round_token_ceiling": 2000}}
        if agent:
            settings["tiers"] = dict(strong=agent, small=agent)
        call(path + "/settings", "PUT", {"revision": 0, "settings": settings})
        call(path, "PUT", {"enabled": True})
        return [s for s in call("/api/schedules")["schedules"] if s["project"] == project]

    def lines():
        return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

    def wait_line(key):
        return wait_for(lambda: next((e for e in lines() if key in e), None), bool, 18, every=.15)

    def seed(project, key, impact, state, at):
        scores = json.dumps(dict(impact=impact, blast=0, reversibility=0, confidence=1.0))
        measure = json.dumps(dict(value=4, baseline=2, threshold=3, window_start=at-30, window_end=at))
        db("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric, "
           "evidence, measure, scores, first_seen, last_seen, count, state) "
           "VALUES (?, ?, 'drift.diff_outlier', 'project', ?, 'diff_size', '[]', ?, ?, ?, ?, 1, ?)",
           (key, project, project, measure, scores, at, at, state))

    try:
        t.start()
        secret = t.signin()
        project, path = create_project("Weekly review")
        quiet, quiet_path = create_project("Quiet project")
        check("AC-242 without a strong agent there is no review schedule", enable(quiet_path, quiet) == [])
        check("AC-243 a quiet project has no exception digest", call(quiet_path + "/screen")["digest"] is None)
        agent = make_agent(project, "Weekly agent")
        [schedule] = enable(path, project, agent)
        check("AC-242 a strong agent with one slot has one weekly Routines schedule",
              schedule["title"] == "Supervisor weekly review" and schedule["cron"] and not schedule["paused"])
        at = int(time.time())
        for n in range(14):
            seed(project, f"signal-{n}", 1, "shadow" if n >= 10 else "raised", at)
        task = call(f"/api/schedules/{schedule['id']}/run", "POST", {})["task"]
        result = wait_line("review")
        context = json.loads(result["review"]["content"][0]["text"])
        report = wait_for(lambda: call(path + "/screen")["review"], bool, 12, every=.15)
        headings = ["Decisions waiting for you", "Top three register problems", "Resolved and recurred",
                    "Effort allocation", "Agent performance", "Owner bottlenecks", "Indicator verdicts",
                    "Supervisor outcomes and budgets", "Verdict batch"]
        order = [report["result"].find(name) for name in headings]
        check("AC-242 the scheduled agent reads its own measured context and publishes the nine sections",
              context["task"] == report["task"] == task and context["project"] == project
              and len(context["indicators"]) == 7 and min(order) == 3 and order == sorted(set(order))
              and result["refused"]["isError"] and not result["finished"]["isError"])
        check("AC-242 the verdict batch is capped and unanswered items have no verdict",
              len(report["items"]) == 10 and all(x["verdict"] is None for x in report["items"])
              and sum(int(x["fingerprint"].split("-")[-1]) >= 10 for x in report["items"]) == 3)
        for key, verdict in zip((x["fingerprint"] for x in report["items"]), ("useful", "not_useful", "wrong")):
            press = dict(task=task, fingerprint=key, verdict=verdict)
            assert call(path + "/verdicts", "POST", press)["verdict"] == verdict
            assert request(quiet_path + "/verdicts", "POST", press)[0] == 400
        check("AC-242 each verdict key records an owner press",
              call(path + "/screen")["review"]["verdict_presses"] == 3)

        incident_project, incident_path = create_project("Incident project")
        talk_agent = make_agent(incident_project, "Incident agent", ["--round-mode", "thread", "--thread-barrier", str(barrier)])
        enable(incident_path, incident_project, talk_agent)
        at = int(time.time())
        seed(incident_project, "incident-signal", 1, "raised", at)
        incident = db("INSERT INTO supervisor_incidents (project, subject_kind, subject_id, opened_at, last_firing_at) "
                      "VALUES (?, 'project', ?, ?, ?) RETURNING id", (incident_project, incident_project, at, at))[0][0]
        db("INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, detector, metric, level, value, threshold) "
           "VALUES (?, ?, 'signal', 'system', 'incident-signal', 'drift.diff_outlier', 'diff_size', 3, 4, 3)", (incident, at))
        base = f"/api/supervisor/incidents/{incident}"
        check("AC-244 Talk refuses an unsigned page and is absent on the agent socket",
              request(base + "/talk", "POST", {}, signed=False)[0] == 401
              and unix_request(t.sock, "POST", base + "/talk", {})[0] == 404)
        talk = call(base + "/talk", "POST", {})
        read = wait_line("talk")
        words = f"Supervisor incident case {read['talk']}. Call get_case_file with its ID and discuss it with the owner."
        data = json.loads(read["content"][0]["text"])
        check("AC-244 Talk types the retained case ID once and starts with the Supervisor role",
              talk["typed"] and data["incident"] == incident and data["kind"] == "conversation"
              and prompts.read_text().splitlines().count(words) == 1
              and any("call as a supervisor" in e.get("probe", "") for e in lines()))
        pane = talk["key"].rsplit(":", 1)[1]
        tmux_on(SOCKET, "kill-pane", "-t", pane)
        wait_for(lambda: db("SELECT state FROM runs WHERE id = ?", [talk["run"]])[0][0],
                 lambda s: s in ("closed", "left"), 12, every=.15)
        call(base + "/investigate", "POST", {})
        wait_for(lambda: any(e.get("tool") == "get_case_file" and not e["isError"] for e in lines()), bool, 15, every=.15)
        thread = call(incident_path + "/incidents")["rows"][0]["task"]
        prose = "Please explain the measured change."
        event = call(f"/api/tasks/{thread}/comments", "POST", {"text": prose})["event"]
        wait_for(lambda: call(f"/api/tasks/{thread}")["history"],
                 lambda rows: any(e["id"] == event and e.get("told") == "waiting" for e in rows), 8, every=.15)
        barrier.touch()
        notice = wait_line("notice")["notice"]
        answer = wait_for(lambda: next((e for e in lines() if e.get("tool") == "post_incident_answer"), None), bool, 12, every=.15)
        history = call(f"/api/tasks/{thread}")["history"]
        check("AC-244 a live investigation hears the ordinary comment notice and answers in the same task",
              not answer["isError"] and thread in notice and prose not in notice and any(e["id"] == event for e in history)
              and any(e.get("text") == "The checked fixture explains the incident." for e in history)
              and prose not in prompts.read_text())
        digest = wait_for(lambda: call(incident_path + "/screen")["digest"], bool, 15, every=.15)
        check("AC-243 new live news has a code digest with a collector heartbeat",
              any(e["fingerprint"] == "incident-signal" for e in digest["entries"]) and digest["last_ok"])
    finally:
        t.cleanup()


if __name__ == "__main__":
    main()
    summary()
