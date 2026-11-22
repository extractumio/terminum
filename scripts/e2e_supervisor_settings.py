#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-220 and AC-221 against a private terminumd and signed-in project routes."""

import json
from pathlib import Path
import sqlite3
import subprocess
import time
from contextlib import closing
from urllib.request import Request, urlopen

from e2e_remote import api
from e2e_ws import Terminumd, check, summary, unix_request, wait_for
from testrun import free_port, own


class Fixture:
    def __init__(self, server):
        self.server = server
        self.session = server.signin()
        self.origin = f"http://127.0.0.1:{server.port}"
        self.headers = {"Content-Type": "application/json", "Origin": self.origin}
        self.db = Path(server.app, "kanban.sqlite")

    def request(self, path, method="GET", body=None, expected=200, session=None):
        status, result = api(self.server, self.session if session is None else session, path, method=method,
                             body=json.dumps(body).encode() if body is not None else None, headers=self.headers)
        assert status == expected, (path, status, result)
        return result

    def export(self, path, **options):
        req = Request(self.origin + path + "/export", method="POST", data=json.dumps(options).encode(),
                      headers={**self.headers, "X-TM-Session": self.session})
        with urlopen(req, timeout=10) as response:
            assert response.headers["Content-Disposition"] == 'attachment; filename="supervisor-settings.json"'
            assert response.headers["Cache-Control"] == "no-store"
            return response.read().decode()

    def project(self, name, wait_minutes, history_days, with_sources=False):
        directory = Path(self.server.root, name.lower())
        directory.mkdir()
        if with_sources:
            subprocess.run(["git", "init", "-q", str(directory)], check=True)
            subprocess.run(["git", "-C", str(directory), "remote", "add", "origin",
                            "https://code.example/source/fixture.git"], check=True)
        project = self.request("/api/task-projects", "POST", {"name": name, "dir": str(directory)})["id"]
        task = self.request("/api/tasks", "POST", {"title": "Historic wait", "project": project})["id"]
        start = int(time.time()) - history_days * 86400
        with closing(sqlite3.connect(self.db)) as db, db:
            db.execute("PRAGMA foreign_keys = ON")
            task = db.execute("SELECT id FROM tasks WHERE slug = ?", (task,)).fetchone()[0]
            for suffix, at, transition in [
                ("start", start, {"from": "TODO", "to": "WAIT FOR HUMAN"}),
                ("end", start + wait_minutes * 60, {"from": "WAIT FOR HUMAN", "to": "TODO"}),
            ]:
                db.execute("INSERT INTO events (id, task, author, at, kind, data) VALUES (?, ?, 'human', ?, 'move', ?)",
                           (f"fixture-{project}-{suffix}", task, at, json.dumps(transition)))
        if with_sources:
            agent = self.request("/api/task-agents", "POST", {
                "project": project, "name": "Stand-in", "kind": "claude", "command": "true",
            })["id"]
            self.request("/api/schedules", "POST", {
                "project": project, "agent": agent, "title": "Paused example",
                "cron": "0 3 * * *", "budget_minutes": 1, "paused": True,
            })
        return f"/api/task-projects/{project}/supervisor"

    def scan(self, path):
        self.request(path, "PUT", {"enabled": True})
        result = wait_for(lambda: self.request(path), lambda s: s["attachment"] is not None, 30, every=0.25)
        return result["attachment"]


def ac221_switch_attaches_two_projects(fixture, api_path, web_path):
    api_scan = fixture.scan(api_path)
    check("AC-221 enabling one project leaves the other off", not fixture.request(web_path)["enabled"])
    web_scan = fixture.scan(web_path)
    waits = [s["detectors"]["flow.wait_human_age"] for s in (api_scan, web_scan)]
    check("AC-221 a second project attaches by switch alone and derives its own historical threshold",
          [d["observed_thresholds"]["wait_minutes"] for d in waits]
          == [{"min": 300, "max": 300}, {"min": 900, "max": 900}]
          and all(d["hold_state"] == "shadow" for d in waits))
    schedule, other = [s["detectors"]["flow.schedule_health"] for s in (api_scan, web_scan)]
    check("AC-221 the scan reports code-only operation, short history, and project-specific sources",
          api_scan["code_only"] and web_scan["code_only"] and not api_scan["limited_history"]
          and web_scan["limited_history"] and api_scan["sources"]["pool_agents"] == 1
          and web_scan["sources"]["pool_agents"] == 0 and api_scan["sources"]["code_host"]
          and not web_scan["sources"]["code_host"] and schedule["can_run"] and not other["can_run"]
          and other["unavailable"] == ["No project routines are configured."])


def ac220_export_redaction_and_import_confirmation(fixture, api_path, web_path):
    r = fixture.request
    view = r(api_path + "/settings")
    settings = view["settings"]
    check("AC-220 defaults are available before a project is configured",
          view["revision"] == 0 and settings["level"] == "operator"
          and all(agent is None for agent in settings["tiers"].values())
          and settings["budgets"]["judgment_percent"] == 3 and settings["budgets"]["fixes_percent"] == 10
          and all(settings["charter"]["indicators"].values()) and view["detector_defaults"])
    r(api_path + "/import", "POST", {"revision": 0, "file": json.dumps({
        "version": 1, "settings": {}, "probe_commands": {"..": "true"},
    })}, expected=400)
    check("AC-220 a path segment cannot become a probe ID",
          r(api_path + "/settings")["revision"] == 0 and not r(api_path + "/settings")["unset_probes"])
    marker = "fixture-" + "placeholder"
    preferences = {"charter": {"priorities": ["Keep builds repeatable"], "mission": f"API_TOKEN={marker}"}}
    r(api_path + "/settings", "PUT", {"revision": 0, "settings": preferences})
    r(api_path + "/settings", "PUT", {"revision": 0, "settings": {}}, expected=400)

    suffix = "/probe-candidates/latency"
    command = "printf '%s\\n' 'probe fixture'\ntrue"
    r(api_path + suffix, "POST", {"command": command})
    approval = r(api_path + suffix + "/preview", "POST", {})
    check("AC-220 the preview returns the entire staged command", approval["command"] == command)
    r(api_path + suffix + "/confirm", "POST", {"approval": approval["approval"]})
    default = fixture.export(api_path)
    with_commands = fixture.export(api_path, include_commands=True)
    with_secrets = fixture.export(api_path, include_secrets=True)
    check("AC-220 default export omits commands and redacts secret shapes; checkboxes are independent",
          "probe_commands" not in json.loads(default) and marker not in default
          and json.loads(with_commands)["probe_commands"]["latency"] == command and marker not in with_commands
          and marker in with_secrets and "probe_commands" not in json.loads(with_secrets))

    r(web_path, "PUT", {"enabled": False})
    imported = r(web_path + "/import", "POST", {"revision": 0, "file": with_commands})
    check("AC-220 import copies preferences, keeps the switch off, and sets no command",
          imported["settings"]["charter"]["priorities"] == ["Keep builds repeatable"]
          and not r(web_path)["enabled"] and imported["confirmed_probes"] == []
          and imported["unset_probes"] == ["latency"]
          and "probe_commands" not in json.loads(fixture.export(web_path)))
    r(web_path + suffix + "/confirm", "POST", {"approval": approval["approval"]}, expected=400)
    old = r(web_path + suffix + "/preview", "POST", {})
    replacement = command + "\n# new candidate"
    r(web_path + suffix, "POST", {"command": replacement})
    r(web_path + suffix + "/confirm", "POST", {"approval": old["approval"]}, expected=400)
    fresh = r(web_path + suffix + "/preview", "POST", {})
    r(web_path + suffix + "/confirm", "POST", {"approval": fresh["approval"]}, expected=401, session="")
    status, _ = unix_request(fixture.server.sock, "POST", web_path + suffix + "/confirm", {"approval": fresh["approval"]})
    assert status == 404, status
    confirmed = r(web_path + suffix + "/confirm", "POST", {"approval": fresh["approval"]})
    check("AC-220 only a fresh signed-in confirmation sets the exact candidate in its own project",
          confirmed["confirmed_probes"] == ["latency"] and not confirmed["unset_probes"]
          and json.loads(fixture.export(web_path, include_commands=True))["probe_commands"] == {"latency": replacement}
          and json.loads(fixture.export(api_path, include_commands=True))["probe_commands"] == {"latency": command})


def main():
    server = Terminumd(free_port(), own("tm-test-supervisor-settings"))
    try:
        server.start()
        fixture = Fixture(server)
        api_path = fixture.project("API", 300, 28, with_sources=True)
        web_path = fixture.project("Web", 900, 10)
        ac221_switch_attaches_two_projects(fixture, api_path, web_path)
        ac220_export_redaction_and_import_confirmation(fixture, api_path, web_path)
    finally:
        server.cleanup()


if __name__ == "__main__":
    main()
    summary()
