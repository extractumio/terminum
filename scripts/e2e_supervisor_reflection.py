#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-248, AC-249 and AC-253: the private Dispatcher admits retained triggers through tmux and MCP."""
from contextlib import closing
import json
from pathlib import Path
import shlex
import sqlite3
import time

from e2e_remote import api
from e2e_ws import BIN, Terminumd, check, summary, wait_for
from testrun import free_port, own

FAKE = Path(__file__).resolve().with_name("fake_agent.py")


def main():
    port, socket = free_port(), own("tm-test-sreflect")
    t = Terminumd(port, socket, env={"TM_TEST_HOOKS": "1", "TM_TEST_CLOCK_SCALE": "60"})
    db_path = Path(t.app, "kanban.sqlite")
    log = Path(t.root, "reflection-tools")
    main_prompts = Path(t.root, "main-prompts")
    prompts = Path(t.root, "reflection-prompts")
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{port}"}

    def db(statement, bindings=()):
        with closing(sqlite3.connect(db_path)) as conn, conn:
            return conn.execute(statement, bindings).fetchall()

    def call(path, method="GET", body=None):
        status, reply = api(t, secret, path, method=method, headers=headers,
                            body=json.dumps(body).encode() if body is not None else None)
        assert status == 200, (path, status, reply)
        return reply

    def lines(file):
        return file.read_text().splitlines() if file.exists() else []

    def agent(project, name, kind, recorded):
        return call("/api/task-agents", "POST", {
            "project": project, "name": name, "kind": kind, "at_once": 1,
            "command": shlex.join(["python3", "-I", str(FAKE), BIN, "--kind", kind, "--prompts", str(recorded),
                                   "--round-log", str(log), "--round-tokens", "200"]),
        })["id"]

    def configure(independent, cap):
        saved = call(path + "/settings")
        pack = saved["settings"]
        pack["tiers"] = dict(strong=main_agent, independent=independent)
        pack["budgets"]["daily_token_ceiling"] = cap
        pack["budgets"]["judgment_floor_tokens"] = 2_000_000
        call(path + "/settings", "PUT", {"revision": saved["revision"], "settings": pack})

    def seed(key, trigger):
        db("INSERT INTO supervisor_reflections (fingerprint,project,generation,subject_kind,subject_id,at,triggers,detail)"
           " SELECT ?,project,generation,'fixture',?,unixepoch(),?,'{}' FROM project_supervisor WHERE project = ?"
           " ON CONFLICT (fingerprint) DO NOTHING",
           ("reflection_" + key, key, json.dumps(trigger), project))

    def ended():
        return db("SELECT q.id,q.case_file,q.finding,r.agent_id,q.token_cap,q.failure"
                  " FROM supervisor_rounds q JOIN runs r ON r.id = q.run"
                  " WHERE r.project = ? AND q.ended_at IS NOT NULL ORDER BY q.rowid", [project])

    try:
        t.start()
        secret = t.signin()
        project = call("/api/task-projects", "POST", {"name": "Reflections", "dir": t.root})["id"]
        path = f"/api/task-projects/{project}/supervisor"
        main_agent = agent(project, "Supervisor fixture", "claude", main_prompts)
        peer = agent(project, "Same family fixture", "claude", main_prompts)
        independent = agent(project, "Reviewer fixture", "codex", prompts)
        configure(main_agent, 0)
        call(path, "PUT", {"enabled": True})
        seed("breaker", ["breaker_opened"])
        waiting = lambda: call(path + "/screen")["reflections"][0]["waiting"]
        check("AC-248 the Supervisor cannot review itself",
              bool(wait_for(waiting, lambda r: r and "other tiers" in r, 12)), waiting())
        configure(peer, 0)
        check("AC-248 another family is preferred when available",
              bool(wait_for(waiting, lambda r: r and "different model family" in r, 12)), waiting())
        configure(independent, 0)
        check("AC-248 an exhausted judgment budget retains the request without starting a run",
              bool(wait_for(waiting, lambda r: r and "budget limit" in r, 12)) and not lines(prompts), waiting())
        triggers = [
            ["human_override"], ["fix_recurred"], ["miss"], ["alert_wrong", "breaker_opened"],
            ["budget_exceeded"], ["decision_rate"], ["incident_postmortem"],
        ]
        for i, trigger in enumerate(triggers):
            seed(str(i), trigger)
            seed(str(i), trigger)
        configure(independent, None)

        # The rounds run one after another on the one independent agent: wait while each next one ends in time,
        # not for one deadline over all of them, which a loaded machine stretches.
        cases = []
        while len(cases) < 1 + len(triggers):
            cases = wait_for(ended, lambda rows, done=len(cases): len(rows) > done, 60, every=.25)
        check("AC-248 every trigger starts exactly one round on the independent agent",
              len(cases) == 8 and all(row[3] == independent and row[4] > 0 and row[5] is None for row in cases)
              and not lines(main_prompts), f"{len(cases)} ended rounds")
        readings = [call(f"{path}/rounds/{case}") for case, *_ in cases]
        check("AC-237 Codex reflection readings reach each closed round exactly once",
              all(row["first_usage_at"] is not None and row["final_usage_at"] is not None
                  and not row["delayed_usage"] and row["measured_tokens"] == 200 for row in readings))
        expected = [f"Supervisor reflection case {case}. Call get_case_file with its ID. Review the code-built timeline independently. "
                    "Explain misses, noise and wrong decisions, and submit configuration changes with replay_config_change and propose_config_change. Cite their change IDs with record_reflection."
                    for case, *_ in cases]
        check("AC-248 the marked pane takes only fixed words and the case ID once", lines(prompts) == expected, lines(prompts))
        tools = [json.loads(line) for line in lines(log) if '"tool":' in line]
        check("AC-248 a reflection cannot change project actions or read another case",
              sum(row.get("isError", False) for row in tools if row["tool"] in ("hold", "run_task", "get_case_file")) == 24,
              [(row["tool"], row.get("isError")) for row in tools])
        check("AC-248 bad evidence is refused and checked proposals are retained",
              sum(row.get("isError", False) for row in tools if row["tool"] == "record_reflection") == 8
              and all(json.loads(row[2])["proposals"][0]["evidence"] == [1] for row in cases))
        screen = call(path + "/screen")
        proposals = {entry["id"]: entry for entry in screen["config_changes"]}
        check("AC-253 each reflection cites a replayed proposal from its own marked session",
              len(proposals) == len(cases) and all(
                  proposals[json.loads(case[2])["proposals"][0]["change_id"]]["class"] == "owner" for case in cases)
              and sum(row.get("isError", False) for row in tools if row["tool"] == "propose_config_change") == 0)

        alert = next(r for r in screen["reflections"] if "alert_wrong" in r["triggers"])
        card = call(f"/api/tasks/{alert['routing_card']}")
        check("AC-249 a routing proposal with missing data goes to a decision card",
              card["status"] == "WAIT FOR HUMAN" and "return unavailable" in card["text"], card["status"])
        count = len(lines(prompts))
        seed("breaker", ["breaker_opened"])
        time.sleep(3)
        check("AC-248 a completed request cannot start again", len(lines(prompts)) == count, lines(prompts))

        seed("config-tune", ["human_override"])
        tuned = wait_for(ended, lambda rows: len(rows) == count + 1, 30, every=.25)
        change = json.loads(tuned[-1][2])["proposals"][0]["change_id"]

        def entry():
            return next(item for item in call(path + "/screen")["config_changes"] if item["id"] == change)

        def detectors():
            return call(path + "/settings")["settings"]["detectors"]

        applied = entry()
        check("AC-253 a marked reflection applies its reproduced threshold tune",
              applied["class"] == "tune" and applied["applied"]
              and detectors()["flow.failed_repeat"]["threshold"] == {"absolute": 2.5})
        call(path + f"/config/{change}/revert", "POST", {})
        check("AC-253 one signed-in revert restores the default and retains the ledger entry",
              entry()["reverted"] and "flow.failed_repeat" not in detectors())
        schedule = next(s for s in call("/api/schedules")["schedules"] if s["title"] == "Supervisor quarterly audit")
        check("AC-259 the project schedules its independent agent quarterly",
              schedule["cron"] == "0 9 1 1,4,7,10 *" and schedule["agent"] == independent, schedule)
        configure(independent, 0)
        denied, _ = api(t, secret, f"/api/schedules/{schedule['id']}/run", method="POST",
                        headers=headers, body=b"{}")
        check("AC-259 budget denies admission and publishes no audit",
              denied == 400 and call(path + "/screen")["audit"]["latest"] is None)
        configure(independent, None)
        task = call(f"/api/schedules/{schedule['id']}/run", "POST", {})["task"]
        audit_rows = lambda: [json.loads(row) for row in lines(log) if '"audit":' in row]
        [audit] = wait_for(audit_rows, bool, 35)
        check("AC-259 the audit reads only its own independent case and reaches the owner unedited",
              all(not reply.get("isError") for reply in audit["audit"].values())
              and audit["denied"]["isError"] and not audit["finished"].get("isError")
              and "settings_revision" not in json.dumps(audit["audit"])
              and call(path + "/screen")["audit"]["latest"]["report"] == audit["report"]
              # its result is a report in the owner's Inbox, its task DONE (terminum-reports.md AC-272, AC-279)
              and call(f"/api/tasks/{task}")["status"] == "DONE"
              and any(r["source"]["task"] == task for r in call("/api/reports")["reports"]), audit)

    except Exception:
        if db_path.exists():
            print("Private reflection state:", db("SELECT triggers,reason,round FROM supervisor_reflections"))
        raise
    finally:
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
