# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-231/232 against the socket, CLI and MCP of a private tmux and terminumd."""

from contextlib import closing
import json
import os
from pathlib import Path
import shlex
import sqlite3

from e2e_ws import BIN, check, wait_for


def ac231_supervisor_tools_pane_and_token(t, call, typed, tool, tmux, page, project, folder, boss, plain, cli, waiting, manager_token):
    state = os.path.join(t.app, "kanban.sqlite")
    settings = f"/api/task-projects/{project}/supervisor/settings"
    switch = f"/api/task-projects/{project}/supervisor"
    binary = shlex.quote(BIN)

    def db(sql, values=()):
        with closing(sqlite3.connect(state)) as conn, conn:
            return conn.execute(sql, values).fetchall()

    def names(out):
        return {name for line in out.splitlines() if line.startswith("  ") for name in line.strip().split(", ")}

    def new(title, where):
        status, task = call("/api/tasks", "POST", {"title": title, "project": where, "text": "Fixture task body"})
        assert status == 200, task
        return task["id"]

    def raw(from_pane, requests):
        lines = []
        for name, args, extra in requests:
            body = {"socket": t.tmux_socket, "pane": pane, "tool": name, "args": args, **extra}
            lines.append(f"curl -sS --unix-socket {shlex.quote(t.sock)} -H 'Content-Type: application/json' "
                         f"-d {shlex.quote(json.dumps(body))} http://localhost/agent")
        script = Path(t.root, "role-requests.sh")
        script.write_text("\necho\n".join(lines) + "\n")
        out, rc = typed(from_pane, f"sh {shlex.quote(str(script))}", 50)
        assert rc == "0", (rc, out)
        return [json.loads(line) for line in out.splitlines()]

    def wire(name, args, **extra):
        return raw(pane, [(name, args, extra)])[0]

    def mcp(*requests):
        lines = [{"jsonrpc": "2.0", "id": 1, "method": "initialize"}]
        lines.extend({"jsonrpc": "2.0", "id": i, "method": method, "params": params}
                     for i, (method, params) in enumerate(requests, 2))
        stdin = shlex.quote("\n".join(map(json.dumps, lines)))
        command = f"printf %s {stdin} | {binary} mcp"
        out, rc = typed(pane, f"sh -c {shlex.quote(command)}")
        assert rc == "0", (rc, out)
        return [json.loads(line) for line in out.splitlines()]

    status, other = call("/api/task-projects", "POST", {"name": "Other", "dir": folder})
    assert status == 200, other
    foreign = new("Foreign task", other["id"])
    local = new("Supervisor comment fixture", project)
    task = new("Supervisor dispatch fixture", project)
    question = new("Supervisor question fixture", project)
    queued, queue = waiting

    page.send({"t": "projectwindow", "project": project, "dir": ""})
    created = page.until(lambda msg: msg["t"] in ("created", "error"), 15) or {}
    key = created["id"]
    pane = key.rsplit(":", 1)[1]
    assert call(switch, "PUT", {"enabled": True})[0] == 200

    # The later Round and Talk launchers own mark creation. Seed their trusted store boundary in this fixture.
    db("INSERT INTO supervisor_panes (pane_key, project, generation) SELECT ?, project, generation FROM project_supervisor WHERE project = ?", (key, project))
    db("UPDATE tasks SET made_in = ? WHERE slug = ?", (key, foreign))
    db("UPDATE tasks SET session_key = ? WHERE slug = ?", (key, question))
    manager, rc = typed(boss, f"{binary} tools")
    catalog = names(manager)
    assert rc == "0" and {"get_task", "send_to_session", "integrate"} <= catalog, manager
    review_context = "get_supervisor_review"
    check("AC-231 the authorization matrix covers all 41 published tools",
          len(catalog) == 41 and review_context in catalog, sorted(catalog))

    # The mark wins even though this pane was opened by the human in the project.
    found = wire("probe", {})
    check("AC-232 an enabled project's default is Operator", found.get("role") == "supervisor" and found.get("level") == "operator", found)
    guessed = raw(plain, [("probe", {}, {"role": "supervisor", "level": "operator"}),
                          ("get_task", {"id": local}, {}),
                          ("probe", {}, {"pane": plain, "role": "supervisor", "level": "operator"})])
    check("AC-231 a request cannot select its role or call as another pane",
          all(got["error"] and "own pane" in got["text"] for got in guessed[:2]) and guessed[2].get("role") == "worker", guessed)
    displaced = wire("update_task", {"id": local, "title": "Unexpected edit"}, pane=plain)
    check("AC-231 a Supervisor cannot switch to worker tools by naming another pane",
          displaced["error"] and "own pane" in displaced["text"], displaced)
    rc, _, err = cli("tokens", "add", "supervision", "--role", "supervisor", "--project", "Managed")
    check("AC-231 the token command offers only manager or worker", rc == 1 and "worker" in err and "manager" in err, err)
    rc, manager_tools, _ = cli("tools", token=manager_token)
    check("AC-231 a manager token discovers the ordinary catalog without the Supervisor review context",
          rc == 0 and "call as a manager" in manager_tools and names(manager_tools) == catalog - {review_context})
    rc, _, _ = cli("tool", review_context, token=manager_token)
    check("AC-242 a token cannot read the review context", rc != 0)
    out, rc = typed(plain, tool(review_context))
    check("AC-242 an unrelated worker cannot read the review context", rc == "1" and "context belongs" in out, out)
    out, rc = typed(boss, tool(review_context))
    check("AC-244 the project's own manager can read its Supervisor context", rc == "0" and project in out, out[:240])

    observer = {"list_tasks", "get_task", "get_case_file", "list_signals", "get_signal", "get_metric", "record_finding"}
    advisor = observer | {"add_comment", "ask_human", "open_problem", "update_problem", "propose_rank", "propose_decision", "post_incident_answer"}
    operator = advisor | {"run_task", "promote_queued_task", "stop_task", "list_runs", "get_dashboard"}
    view = call(settings)[1]

    for level, allowed in [("Observer", observer), ("Advisor", advisor), ("Operator", operator)]:
        status, view = call(settings, "PUT", {"revision": view["revision"], "settings": {**view["settings"], "level": level.lower()}})
        assert status == 200, view
        out, rc = typed(pane, f"{binary} tools")
        check(f"AC-231 CLI discovery returns exactly the {level} allow-list", rc == "0" and "call as a supervisor" in out and names(out) == allowed, out)
        requests = [
            ("tools/list", {}),
            ("tools/call", {"name": "get_tool_info", "arguments": {"names": ["get_session"]}}),
            ("tools/call", {"name": "execute_tool", "arguments": {"name": "list_tasks"}}),
            ("tools/call", {"name": "execute_tool", "arguments": {"name": "update_task", "arguments": {"id": local}}}),
        ]
        if level == "Operator":
            requests.append(("tools/call", {"name": "get_tool_info", "arguments": {"names": ["run_task"]}}))
        init, listing, refused, reading, writing, *start = mcp(*requests)
        check(f"AC-231 MCP advertises only discovery and execution wrappers for {level}",
              "Supervisor" in init["result"]["instructions"] and
              {d["name"] for d in listing["result"]["tools"]} == {"search_tools", "get_tool_info", "execute_tool"} and
              refused["result"]["isError"], (init, refused))
        check(f"AC-231 execute_tool calls allowed tools and refuses other tools at {level}",
              not reading["result"]["isError"] and writing["result"]["isError"]
              and writing["result"]["content"][0]["text"] == f'The {level} Supervisor cannot call "update_task".')
        if start:
            card = json.loads(start[0]["result"]["content"][0]["text"])[0]
            check("AC-231 the Operator's run_task schema and example use only task ID and agent",
                  set(card["inputSchema"]["properties"]) == {"id", "agent"}
                  and set(card["inputSchema"]["required"]) == {"id", "agent"}
                  and set(card["example"]["arguments"]["arguments"]) == {"id", "agent"})

        requests = sorted(catalog)
        answers = raw(pane, [(name, {}, {}) for name in requests])
        assert len(answers) == len(requests)
        for name, answer in zip(requests, answers):
            refused = f'The {level} Supervisor cannot call "{name}".'
            check(f"AC-231 {level} reaches {name}'s own validation" if name in allowed else f"AC-231 {level} is refused {name}",
                  answer.get("role") == "supervisor" and
                  ((not answer["error"] or answer["text"] in {"id is required", "title is required"}) if name in allowed
                   else answer["error"] and answer["text"] == refused), answer["text"] if answer["error"] else "")

        if level == "Observer":
            listed = json.loads(wire("list_tasks", {})["text"].splitlines()[1])
            result = wire("get_task", {"id": foreign})
            check("AC-231 project scope wins over a foreign task's made-in pane",
                  local in {row["id"] for row in listed} and foreign not in {row["id"] for row in listed}
                  and result["error"] and "outside this session's project" in result["text"], result)
        if level == "Advisor":
            edited = wire("add_comment", {"id": foreign, "text": "Fixture should not be written"})
            check("AC-231 Advisor cannot comment outside the bound project", edited["error"] and "outside" in edited["text"], edited)
            comment = wire("add_comment", {"id": local, "text": "The fixture has been reviewed."})
            check("AC-231 Advisor comments in its project under its own role",
                  not comment["error"] and any(e["author_name"] == "Supervisor" for e in call(f"/api/tasks/{local}")[1]["history"]), comment)
            made = wire("create_task", {"title": "Feat: Advisor fixture card", "text": "Fixture", "severity": "P1"})
            check("AC-238 Advisor cannot bypass the ranked register with create_task", made["error"], made)
            empty = wire("open_problem", {"title": "Fixture problem", "lens": "flow", "hypothesis": "A repeated failure.",
                                         "disproof": "The check passes twice.", "effort": 1, "confidence": 0.8,
                                         "fix_options": [], "signals": []})
            check("AC-236 Advisor cannot open a problem with no signal", empty["error"], empty)
            questions = "Why you are needed: choose the fixture's format.\nRecommended:\n1. Text: easy to read. Other options: table.\nDetails: both fixtures cover the case."
            asked = wire("ask_human", {"id": question, "questions": questions})
            check("AC-231 Advisor can ask on its own task", not asked["error"] and call(f"/api/tasks/{question}")[1]["status"] == "WAIT FOR HUMAN", asked)

    for option in ("comment", "fresh", "queue"):
        answer = wire("run_task", {"id": task, "agent": "Fake", option: "fixture"})
        check(f"AC-231 Supervisor run_task rejects {option}", answer["error"] and "takes only a task ID and an agent" in answer["text"], answer)
    cli_args, rc = typed(pane, tool("run_task", id=task, agent="Fake", comment="fixture"))
    check("AC-231 CLI preserves the Supervisor's run_task argument restriction",
          rc == "1" and "takes only a task ID and an agent" in cli_args, cli_args)
    # Private fixture: age the human's queue action and supply a checked, high-impact problem.
    db("UPDATE events SET at = at - 86401 WHERE task = (SELECT id FROM tasks WHERE slug = ?)", [queued])
    goal = db("SELECT id FROM tasks WHERE slug = ?", [task])[0][0]
    why = db("SELECT id FROM events WHERE task = ? AND kind = 'made'", [goal])[0][0]
    db("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id, metric, evidence, "
       "measure, scores, first_seen, last_seen, count, state) VALUES ('role-fixture', ?, 'flow.failed_repeat', 'task', ?, "
       "'failed_runs', ?, ?, ?, 1, 1, 1, 'new')", (
           project, goal, json.dumps([{"table": "events", "id": why}]),
           json.dumps({"value": 3, "threshold": 2, "baseline": 2, "window_start": 0, "window_end": 1}),
           json.dumps({"impact": 2, "blast": 0, "reversibility": 0, "confidence": 1.0}),
       ))
    p = db("INSERT INTO supervisor_problems (project, title, lens, hypothesis, disproof, options, confidence, effort, state, opened_at) "
           "VALUES (?, 'Role fixture', 'flow', 'The check fails twice.', 'It passes from a clean store.', '[]', 1, 1, 'confirmed', 1) "
           "RETURNING id", [project])[0][0]
    db("INSERT INTO supervisor_problem_signals (problem, fingerprint, threshold, weeks) VALUES (?, 'role-fixture', 2, 4)", [p])
    ranked = wire("propose_rank", {"problem": p, "task": task})
    check("AC-239 the checked tool proposes a rank without editing a human's task",
          not ranked["error"] and call(f"/api/tasks/{task}")[1]["title"] == "Supervisor dispatch fixture", ranked)
    pack = view["settings"]
    pack["budgets"]["fixes_ceiling"] = 1
    status, view = call(settings, "PUT", {"revision": view["revision"], "settings": pack})
    assert status == 200, view
    promoted = wire("promote_queued_task", {"id": queued})
    check("AC-231 Operator promotes an existing wait",
          not promoted["error"] and queue()[0] == queued, promoted)

    started = wire("run_task", {"id": task, "agent": "Fake"})
    check("AC-231 Operator starts the task through Dispatcher", not started["error"], started)
    def delivered():
        return next((text for text, in db("SELECT text FROM inputs WHERE kind = 'prompt' AND sender = 'dispatcher'")
                     if f"Supervisor started task {task}" in text), "")
    prompt = wait_for(delivered, bool, 15)
    check("AC-231 Dispatcher types fixed words with the task ID and no stored task text",
          prompt and "Supervisor dispatch fixture" not in prompt and "Fixture task body" not in prompt, prompt)
    run_pane = wait_for(lambda: call(f"/api/tasks/{task}")[1].get("session"), bool, 10)["key"].rsplit(":", 1)[1]
    stopped = wire("stop_task", {"id": task})
    panes = tmux("list-panes", "-a", "-F", "#{pane_id}").split()
    check("AC-231 Operator's Give up frees the run and leaves its window",
          not stopped["error"] and run_pane in panes and call(f"/api/tasks/{task}")[1]["status"] == "FAILED", stopped)

    status, _ = call(f"/api/tasks/{task}", "PATCH", {"status": "TODO"})
    assert status == 200
    overridden = wire("propose_rank", {"problem": p, "task": task})
    check("AC-239 the owner's move immediately blocks a fresh rank and records self.fights",
          overridden["error"] and "24 hours" in overridden["text"]
          and db("SELECT count FROM supervisor_signals WHERE project = ? AND detector = 'self.fights' AND subject_id = ?",
                 [project, task]) == [(1,)], overridden)

    status, view = call(settings, "PUT", {"revision": view["revision"], "settings": {**view["settings"], "level": "observer"}})
    assert status == 200, view
    cached = wire("run_task", {"id": local, "agent": "Fake"})
    check("AC-231 AC-232 the next call from a cached Operator observes the owner's downgrade",
          cached["error"] and cached["text"] == 'The Observer Supervisor cannot call "run_task".', cached)

    assert call(switch, "PUT", {"enabled": False})[0] == 200
    assert call(switch, "PUT", {"enabled": True})[0] == 200
    retired = wire("update_task", {"id": local, "title": "Unexpected edit"})
    check("AC-231 a pane from the old generation stays restricted after re-enable",
          retired.get("role") == "supervisor" and retired["error"] and retired["text"] == "This Supervisor session is inactive.", retired)
    tmux("kill-pane", "-t", pane)
