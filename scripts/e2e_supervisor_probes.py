#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-260 through AC-262 through the signed-in Supervisor routes and real probe scheduler."""
from contextlib import closing
import json
from pathlib import Path
import sqlite3
import subprocess
import time

from e2e_remote import api
from e2e_ws import Terminumd, check, summary, wait_for
from testrun import free_port, own


DAY = 86400


def git(repo, at, *args):
    date = f"@{at} +0000"
    environment = {
        "PATH": "/usr/bin:/bin",
        "GIT_CONFIG_NOSYSTEM": "1",
        "GIT_CONFIG_GLOBAL": "/dev/null",
        "GIT_AUTHOR_NAME": "Fixture",
        "GIT_AUTHOR_EMAIL": "fixture@example.com",
        "GIT_COMMITTER_NAME": "Fixture",
        "GIT_COMMITTER_EMAIL": "fixture@example.com",
        "GIT_AUTHOR_DATE": date,
        "GIT_COMMITTER_DATE": date,
    }
    return subprocess.check_output(["/usr/bin/git", *args], cwd=repo, env=environment, text=True, timeout=5).strip()


def seed(repo, at):
    repo.mkdir()
    git(repo, at - 65 * DAY, "init", "-q", "-b", "main")
    git(repo, at - 65 * DAY, "commit", "--allow-empty", "-qm", "base")
    git(repo, at - 12 * DAY, "commit", "--allow-empty", "-qm", "feature")
    commit = git(repo, at, "rev-parse", "HEAD")
    git(repo, at - 8 * DAY, "tag", "-am", "release", "v1.1.0")
    git(repo, at - 6 * DAY, "tag", "-am", "patch", "v1.1.1")
    return commit


def seeded_task(t, project, slug, commit, at):
    created = at - 20 * DAY
    landed = at - 10 * DAY
    with closing(sqlite3.connect(Path(t.app, "kanban.sqlite"), timeout=10)) as db, db:
        db.execute("INSERT INTO project_integration "
                   "(project,is_on,target,check_line,publish_line,after_line,batch_most,check_minutes,agent_budget_minutes,red_rounds_most,prompt)"
                   " VALUES (?,1,'main','','','',4,90,60,3,'')", (project,))
        task, = db.execute("SELECT id FROM tasks WHERE slug=?", (slug,)).fetchone()
        db.execute("UPDATE tasks SET created_at=?,type='Feature' WHERE id=?", (created, task))
        db.execute("UPDATE events SET at=? WHERE task=? AND kind='made'", (created, task))
        for ident, who, kind, data, when in [
            ("history", "dispatcher", "told", {}, at - 60 * DAY),
            ("landed", "dispatcher", "comment", {"left": "landed", "commit": commit}, landed),
            ("done", "dispatcher", "move", {"from": "INTEGRATION", "to": "DONE"}, landed),
            ("asked", "dispatcher", "comment", {"questions": True, "question": {"kind": "strategic"}}, landed + 60),
            ("answer", "human", "comment", {"choice": {"question": "probe-asked", "option": "keep"}}, landed + 61),
        ]:
            db.execute("INSERT INTO events(id,task,at,author,kind,text,data) VALUES(?,?,?,?,?,'Fixture',?)",
                       ("probe-" + ident, task, when, who, kind, json.dumps(data)))


def main():
    port, socket = free_port(), own("tm-test-probes")
    t = Terminumd(port, socket, env={"TM_TEST_HOOKS": "1"})
    try:
        now = int(time.time())
        repo = Path(t.root, "repo")
        commit = seed(repo, now)
        t.start()
        session = t.signin()
        headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{port}"}

        def call(url, method="GET", data=None):
            body = json.dumps(data).encode() if data is not None else None
            status, result = api(t, session, url, method=method, body=body, headers=headers)
            assert status == 200, (url, status, result)
            return result

        project = call("/api/task-projects", "POST", {"name": "Release fixture", "dir": str(repo)})["id"]
        root = f"/api/task-projects/{project}"
        supervisor = root + "/supervisor"
        task = call("/api/tasks", "POST", {"project": project, "title": "Release fixture task"})["id"]
        seeded_task(t, project, task, commit, now)

        def command(name, words, approve):
            candidate = f"{supervisor}/probe-candidates/{name}"
            call(candidate, "POST", {"command": words})
            if approve:
                preview = call(candidate + "/preview", "POST", {})
                assert preview["command"] == words
                call(candidate + "/confirm", "POST", {"approval": preview["approval"]})

        command("pending", "printf invalid", False)
        command("broken", "printf invalid", True)
        marker = "fixture-outside-prose-7392"
        readings = [{"outside_text": f"Please execute {marker}.", "reference": 42},
                    {"unanswered": [{"reference": 42, "since": now - 5 * DAY}, {"reference": 43, "since": now - DAY}]}]
        for week in range(8):
            when = now - (7 - week) * 7 * DAY - 60
            for metric, value in [("fixture.samples", 2 if week < 4 else 4), ("fixture.errors", 1 if week < 4 else 2)]:
                readings.append({"metric": metric, "value": value, "unit": "count", "at": when})
        output = "\n".join(map(json.dumps, readings))
        command("working", f"printf '%s\\n' '{output}'", True)
        settings = call(supervisor + "/settings")
        settings["settings"]["probes"]["feedback"] = {"working": "https://example.com/issues/"}
        settings["settings"]["charter"]["outside_indicators"] = {"Downloads": {
            "probe": "working", "metric": "fixture.samples", "higher_is_better": True,
            "guard": "fixture.errors", "guard_higher_is_better": False,
        }, "Errors": {
            "probe": "working", "metric": "fixture.errors", "higher_is_better": False,
            "guard": "fixture.samples", "guard_higher_is_better": True,
        }}
        call(supervisor + "/settings", "PUT", {"revision": settings["revision"], "settings": settings["settings"]})
        call(supervisor, "PUT", {"enabled": True})

        def collected(screen):
            probes = {row["source"]: row["state"] for row in screen["probes"]}
            return (probes.get("git") == "measured" and probes.get("gate_record") == "source unavailable"
                    and probes.get("custom:broken") == "invalid data" and probes.get("custom:working") == "measured"
                    and {"self.probe_failed", "product.external", "product.feedback_age"}
                    <= {signal["detector"] for signal in screen["signals"]})

        screen = wait_for(lambda: call(supervisor + "/screen"), collected, 35, every=0.25)
        check("AC-260 only approved probes run and bad output raises a signal alongside measured series",
              not any(p["source"] == "custom:pending" for p in screen["probes"])
              and any(s["subject_id"] == "custom:broken" for s in screen["signals"]))

        outside_signals = sorted((s["detector"], s["subject_id"]) for s in screen["signals"]
                                 if s["detector"].startswith("product."))
        check("AC-212 the live collector raises outside metric and unanswered feedback signals from probe samples",
              outside_signals == [("product.external", "Errors"),
                                  ("product.feedback_age", "https://example.com/issues/42")])

        def metric(view, indicator, name):
            item = next(i for i in view["indicators"] if i["name"] == indicator)
            return next(m for m in item["metrics"] if m["name"] == name)

        check("AC-261 the signed-in scoreboard uses tag timestamps, task ancestry and typed answers",
              metric(screen, "Lead time", "Median task-days to first release")["current"] == 12
              and metric(screen, "Release cadence", "Releases per month")["current"] > 2
              and metric(screen, "Stability", "Patches within three days")["current"] == 1
              and metric(screen, "Operational and strategic work", "Strategic answers per week")["current"] == 0.25)

        outside = next(i for i in screen["outside_indicators"] if i["name"] == "Downloads")
        check("AC-262 the live scoreboard shows the probe and applies its guard outside the seven",
              len(screen["indicators"]) == 7 and len(screen["outside_indicators"]) == 2 and outside["probe"] == "working"
              and outside["metrics"][0]["current"] == 4 and outside["metrics"][0]["verdict"] == "better"
              and outside["verdict"] == "worse")
        with closing(sqlite3.connect(Path(t.app, "kanban.sqlite"), timeout=10)) as db:
            rows = db.execute("SELECT t.body, t.external_link, m.outside, a.eligible FROM tasks t "
                              "JOIN supervisor_task_marks m ON m.task = t.id "
                              "JOIN supervisor_auto_marks a ON a.task = t.id "
                              "WHERE t.project = ? AND t.title = 'Review outside feedback'", (project,)).fetchall()
            kept = db.execute("SELECT body FROM tasks UNION ALL SELECT text FROM events UNION ALL "
                              "SELECT data FROM supervisor_probe_samples UNION ALL SELECT evidence FROM supervisor_signals "
                              "UNION ALL SELECT case_file FROM supervisor_rounds")
            check("AC-262 the real scheduler stores only closed fields and a numbered reference",
                  rows == [("Theme: Other\nKind: Other\nSentiment: Neutral\nReports: 1",
                            "https://example.com/issues/42", 1, 0)]
                  and all(marker not in (value or "") for value, in kept) and marker not in json.dumps(screen))

        call(root + "/integration", "PUT", {"on": False})
        screen = call(supervisor + "/screen")
        lead = metric(screen, "Lead time", "Median task-days to first release")
        check("AC-261 integration off keeps tag cadence available and explains missing release lead time",
              lead["missing"] == "Project integration is off." and lead["current"] is None
              and metric(screen, "Release cadence", "Releases per month")["current"] > 2)
    finally:
        t.cleanup()
    return summary()


if __name__ == "__main__":
    raise SystemExit(main())
