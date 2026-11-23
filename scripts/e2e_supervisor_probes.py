#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-260 and AC-261 through the signed-in Supervisor routes and real probe scheduler."""
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
        reading = json.dumps({"metric": "fixture.samples", "value": 2, "unit": "count", "at": now})
        command("working", f"printf '%s\\n' '{reading}'", True)
        call(supervisor, "PUT", {"enabled": True})

        def collected(screen):
            probes = {row["source"]: row["state"] for row in screen["probes"]}
            return (probes.get("git") == "measured" and probes.get("gate_record") == "source unavailable"
                    and probes.get("custom:broken") == "invalid data" and probes.get("custom:working") == "measured"
                    and any(signal["detector"] == "self.probe_failed" for signal in screen["signals"]))

        screen = wait_for(lambda: call(supervisor + "/screen"), collected, 35, every=0.25)
        check("AC-260 only approved probes run and bad output raises a signal alongside measured series",
              not any(p["source"] == "custom:pending" for p in screen["probes"])
              and any(s["subject_id"] == "custom:broken" for s in screen["signals"]))

        def metric(view, indicator, name):
            item = next(i for i in view["indicators"] if i["name"] == indicator)
            return next(m for m in item["metrics"] if m["name"] == name)

        check("AC-261 the signed-in scoreboard uses tag timestamps, task ancestry and typed answers",
              metric(screen, "Lead time", "Median task-days to first release")["current"] == 12
              and metric(screen, "Release cadence", "Releases per month")["current"] > 2
              and metric(screen, "Stability", "Patches within three days")["current"] == 1
              and metric(screen, "Operational and strategic work", "Strategic answers per week")["current"] == 0.25)

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
