#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-248 and AC-249: the private Dispatcher admits retained triggers through tmux and MCP."""
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
                                   "--round-log", str(log)]),
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
        main_agent = agent(project, "Supervisor fixture", "codex", main_prompts)
        peer = agent(project, "Same family fixture", "codex", main_prompts)
        independent = agent(project, "Reviewer fixture", "claude", prompts)
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

        cases = wait_for(ended, lambda rows: len(rows) == 1 + len(triggers), 100, every=.25)
        check("AC-248 every trigger starts exactly one round on the independent agent",
              len(cases) == 8 and all(row[3] == independent and row[4] > 0 and row[5] is None for row in cases)
              and not lines(main_prompts), f"{len(cases)} ended rounds")
        expected = [f"Supervisor reflection case {case}. Call get_case_file with its ID. Review the code-built timeline independently. "
                    "Explain misses, noise and wrong decisions, and submit proposals with record_reflection."
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
        alert = next(r for r in screen["reflections"] if "alert_wrong" in r["triggers"])
        card = call(f"/api/tasks/{alert['routing_card']}")
        check("AC-249 a routing proposal with missing data goes to a decision card",
              card["status"] == "WAIT FOR HUMAN" and "return unavailable" in card["text"], card["status"])
        count = len(lines(prompts))
        seed("breaker", ["breaker_opened"])
        time.sleep(3)
        check("AC-248 a completed request cannot start again", len(lines(prompts)) == count, lines(prompts))
    except Exception:
        if db_path.exists():
            print("Private reflection state:", db("SELECT triggers,reason,round FROM supervisor_reflections"))
        raise
    finally:
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
