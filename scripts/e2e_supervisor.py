#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-216–219, AC-227–229 and terminum-reports.md AC-276 against private terminumd, signed-in board routes and tmux."""
import json
import os
from pathlib import Path
import shlex
import sqlite3
import time
from contextlib import closing

from e2e_remote import api
from e2e_ws import Terminumd, check, summary, unix_request, wait_for
from supervisor_report_checks import ac276_a_critical_report_opens_an_incident
from testrun import free_port, own

PORT, SOCKET = free_port(), own("tm-test-supervisor")


def critical_line(t, api_path, web_path, project, call, headers):
    line, test = api_path + "/line", api_path + "/line/test"
    failing, working = Path(t.root, "first channel"), Path(t.root, "second channel")
    output = Path(t.root, "line-notices")
    failing.write_text("#!/bin/sh\ncat >/dev/null\nexit 17\n")
    working.write_text("#!/bin/sh\ncat >> " + shlex.quote(str(output)) + "\n")
    for executable in (failing, working):
        executable.chmod(0o700)
    settings = dict(channels=[str(failing), str(working)], link_base="https://terminum.example", quiet=None, ladder=[10, 30, 60])
    saved = {"revision": 0, "settings": settings}
    check("AC-227 both line writes require the signed-in page listener",
          api(t, "", line, method="PUT", body=json.dumps(saved).encode(), headers=headers)[0] == 401
          and api(t, "", test, method="POST", body=b'{"channel":1,"revision":1}', headers=headers)[0] == 401
          and unix_request(t.sock, "PUT", line, saved)[0] == 404
          and unix_request(t.sock, "POST", test, {"channel": 1, "revision": 1})[0] == 404)
    configured = call(line, "PUT", saved)
    check("AC-227 line settings are shared across projects and excluded from exported project packs",
          configured == call(web_path + "/line")
          and "channels" not in json.dumps(call(web_path + "/export", "POST", {})))
    check("AC-229 the signed-in test sends fixed bytes through the selected channel",
          call(test, "POST", {"channel": 1, "revision": configured["revision"]}) == {"sent": True}
          and output.read_bytes() == b"Terminum critical line test alert.\n")

    def notices():
        return output.read_text() if output.exists() else ""

    def incident():
        return next((entry for entry in call("/api/supervisor/incidents")
                     if entry["project"] == project and "missed" in entry["happened"]), None)

    return failing, notices, incident

def main():
    t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1"})
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"}
    path = os.path.join(t.app, "kanban.sqlite")

    def read(sql, args=()):
        with closing(sqlite3.connect(f"file:{path}?mode=ro", uri=True)) as db:
            return db.execute(sql, args).fetchall()

    def signals(project):
        return dict(read("SELECT detector, count FROM supervisor_signals WHERE project = ?", (project,)))

    def write(sql):
        with closing(sqlite3.connect(path)) as db, db:
            db.execute(sql)

    try:
        t.start()
        session = t.signin()

        def call(url, method="GET", data=None):
            body = json.dumps(data).encode() if data is not None else None
            status, result = api(t, session, url, method=method, body=body, headers=headers)
            assert status == 200, (url, status, result)
            return result

        api_id, web_id = [call("/api/task-projects", "POST", {"name": name, "dir": t.root})["id"] for name in ("API", "Web")]
        api_path, web_path = [f"/api/task-projects/{project}/supervisor" for project in (api_id, web_id)]

        def fails(project, start):
            for i in range(start, start + 2):
                task = call("/api/tasks", "POST", {"title": f"Test card {i}", "project": project})["id"]
                call(f"/api/tasks/{task}", "PATCH", {"status": "FAILED"})

        fails(web_id, 1)
        check("AC-216 both projects start off and off writes no runtime rows",
              not call(api_path)["enabled"] and not call(web_path)["enabled"] and not read("SELECT * FROM project_supervisor")
              and not read("SELECT * FROM supervisor_changes") and not read("SELECT * FROM supervisor_points"))
        unauthorized, _ = api(t, "", web_path, method="PUT", body=b'{"enabled":true}', headers=headers)
        socket_status, _ = unix_request(t.sock, "PUT", web_path, {"enabled": True})
        check("AC-216 the switch requires a signed-in page", unauthorized == 401 and socket_status == 404)

        page = t.page(session)
        page.send({"t": "projectwindow", "project": api_id, "dir": ""})
        pane_key = page.until(lambda m: m.get("t") == "created", 15)["id"]
        pane = pane_key.rsplit(":", 1)[1]
        call(api_path, "PUT", {"enabled": True})
        wait_for(lambda: call(api_path), lambda s: s["last_ok"] is not None, 15)
        start = time.monotonic()
        fails(api_id, 3)
        live = wait_for(lambda: signals(api_id), lambda s: "flow.failed_no_reason" in s, 59, every=0.25)
        check("AC-217 an enabled project's board change shows a signal within one minute",
              time.monotonic() - start < 60 and live["flow.failed_no_reason"] > 0 and not signals(web_id),
              f"{time.monotonic() - start:.1f} seconds")
        check("AC-218 the sweep keeps present measurements only for the on project",
              read("SELECT project, value FROM supervisor_points WHERE metric = 'queue_waits'") == [(api_id, 0)])

        for event in ["SessionStart", "PreToolUse", "PreToolUse", "PreToolUse", "PreToolUse", "PreCompact"]:
            status, _ = unix_request(t.sock, "POST", "/hook", {"agent": "claude", "session": "fixture-session",
                "event": event, "socket": SOCKET, "pane": pane, "tool_call_hash": "abcdef1234567890", "output": "discard this payload"})
            assert status in (200, 204), status
        counters = lambda: dict(read("SELECT event, sum(count) FROM supervisor_hook_counts WHERE project = ? AND session = ? GROUP BY event",
                                     (api_id, "fixture-session")))
        got = wait_for(counters, lambda c: c.get("same_call") == 3, 30)
        check("AC-218 accepted hooks keep numerical session counts and the detector uses them",
              got == {"compaction": 1, "same_call": 3, "tool": 4} and wait_for(
                  lambda: signals(api_id), lambda s: "drift.loop" in s, 30, every=0.25))
        ac276_a_critical_report_opens_an_incident(t, SOCKET, pane, call, read)

        page.send({"t": "project", "action": "add", "id": pane_key, "project": web_id})
        wait_for(lambda: read("SELECT project FROM project_members WHERE pane_key = ?", (pane_key,)), lambda row: row == [(web_id,)], 10)
        unix_request(t.sock, "POST", "/hook", {"agent": "claude", "session": "fixture-session",
            "event": "PreToolUse", "socket": SOCKET, "pane": pane, "tool_call_hash": "abcdef1234567890"})
        beat = call(api_path)["last_ok"]
        wait_for(lambda: call(api_path), lambda s: s["last_ok"] > beat, 20, every=0.25)
        check("AC-218 moving a pane to an off project stops counting its hooks in the old project", counters() == got)

        failing, notices, incident = critical_line(t, api_path, web_path, api_id, call, headers)

        # Fail the collector's commit without stopping the server or its independent writer.
        write("""CREATE TRIGGER stall_supervisor BEFORE UPDATE OF last_ok ON project_supervisor
                 BEGIN SELECT RAISE(ABORT, 'test collector stopped'); END""")
        last_ok = call(api_path)["last_ok"]
        fault = wait_for(lambda: call(api_path), lambda s: s["collector_dead"], 60, every=0.25)
        check("AC-219 the independent watchdog records and exposes a stopped collector",
              fault["last_ok"] == last_ok and signals(api_id).get("self.collector_dead") == 1 and not signals(web_id))
        opened = wait_for(incident, lambda entry: entry is not None, 15)
        incident_id = opened["id"]
        alert = (f"Critical incident {incident_id} · Incident level. Active signals: 1. Acknowledge in Terminum.\n"
                 f"https://terminum.example/supervisor/?project={api_id}&incident={incident_id}#incident-{incident_id}\n")
        wait_for(notices, lambda text: alert in text, 20)
        health = wait_for(lambda: call("/api/supervisor/incidents"),
                          lambda entries: any("Channel 1: Exit status 17." in i["happened"] for i in entries), 15)
        check("AC-227 AC-229 a real watchdog incident falls through to the working command and stays locally actionable",
              alert in notices() and any("Channel 1: Exit status 17." in i["happened"] for i in health)
              and signals(api_id).get("self.line_dead") == 1)
        write("DROP TRIGGER stall_supervisor")
        wait_for(lambda: call(api_path), lambda s: not s["collector_dead"] and s["last_ok"] > last_ok, 20, every=0.25)
        check("AC-219 a successful collector resolves the previous fault",
              read("SELECT state FROM supervisor_signals WHERE project = ? AND detector = 'self.collector_dead'", (api_id,)) == [("resolved",)])

        all_clear = f"All-clear on incident {incident_id}.\n"
        check("AC-227 the watchdog recovery sends the all-clear for the same incident",
              all_clear in wait_for(notices, lambda text: all_clear in text, 20))
        failing.write_text("#!/bin/sh\ncat >/dev/null\n")
        retest = call(api_path + "/line/test", "POST", {"channel": 0, "revision": 1})
        check("AC-229 a successful retest clears the local line failure",
              retest == {"sent": True} and read("SELECT state FROM supervisor_signals WHERE project = ? AND detector = 'self.line_dead'", (api_id,)) == [("resolved",)])

        call(api_path, "PUT", {"enabled": False})
        at_off = call(api_path)["last_ok"]
        fails(api_id, 5)
        check("AC-216 switching off leaves past observations and stops new writes",
              not call(api_path)["enabled"] and call(api_path)["last_ok"] == at_off
              and not read("SELECT * FROM supervisor_changes") and not call(web_path)["enabled"])
        page.close()
    finally:
        t.cleanup()


if __name__ == "__main__":
    main()
    summary()
