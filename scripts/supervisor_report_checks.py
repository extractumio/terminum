# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""terminum-reports.md AC-276 against a private terminumd: a critical report made from a pane of a project whose
Supervisor is on opens an incident that alerts once and links it; the Inbox's read acknowledges that incident."""
import json
import os
import shlex

from e2e_ws import BIN, check, tmux_on, wait_for


def ac276_a_critical_report_opens_an_incident(t, socket, pane, call, read):
    def typed(command):
        out = os.path.join(t.root, "report-tool")
        line = f"TM_HOME={shlex.quote(t.home)} TM_APP_DIR={shlex.quote(t.app)} {command} > {out}.txt 2>&1; echo $? > {out}.rc"
        tmux_on(socket, "send-keys", "-t", pane, "-l", line)
        tmux_on(socket, "send-keys", "-t", pane, "Enter")

        def status():
            try:
                return open(out + ".rc").read().strip()     # the shell makes the file, then writes it
            except OSError:
                return ""
        rc = wait_for(status, bool, 20, 0.2, last=True) or "no answer"
        try:
            return open(out + ".txt").read(), rc
        except OSError:
            return "", rc

    args = {"title": "Build broken", "summary": "main is red", "body": "## What\nThe build fails.", "critical": "yes"}
    out, rc = typed(f"{shlex.quote(BIN)} tool create_report {shlex.quote(json.dumps(args))}")
    report = out.split()[0] if rc == "0" and out else ""
    linked = wait_for(lambda: call("/api/supervisor/incidents"),
                      lambda entries: any(report in i.get("reports", []) for i in entries), 15) or []
    found = next((i for i in linked if report in i.get("reports", [])), None)
    check("AC-276 a critical report under the Supervisor opens a level 3 incident that links it, its own line down",
          found is not None and found["level"] == 3 and found["happened"].startswith("A critical report is unread.")
          and read("SELECT line, alerted_by FROM reports WHERE slug = ?", (report,)) == [(0, found["id"])]
          and read("SELECT count(*) FROM reports WHERE incident = ?", (found["id"],)) == [(0,)], (out, rc, linked))
    if found is None:
        return
    call(f"/api/reports/{report}/read", "POST", {})
    acknowledged = wait_for(lambda: call("/api/supervisor/incidents"),
                            lambda entries: all(i["id"] != found["id"] or i["acknowledged_at"] for i in entries), 10)
    check("AC-276 reading the report in the Inbox acknowledges its incident, by the owner",
          acknowledged is not None
          and read("SELECT actor FROM supervisor_incident_events WHERE incident = ? AND kind = 'acknowledged'", (found["id"],)) == [("human",)])
