#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Roles of terminum's tools (terminum-agents.md AC-174, AC-178..AC-181; terminum-schedules.md AC-182) against a
private terminumd and tmux: the session a + Session started is its project's manager and runs the manager's
tools by `terminum tool`, typed in its own pane; a pane with no run is a worker and is refused, also when it
names the manager's pane; tokens are made outside every pane and give a program a role; a manager starts a task
on an agent (scripts/fake_agent.py), in its own words; types into and closes another session of its project;
runs a schedule now.

    python3 scripts/e2e_roles.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import shlex
import sqlite3
import subprocess
import sys
import tempfile
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import BIN, Terminumd, check, summary, tmux_on, wait_for, wait_layout  # noqa: E402
from e2e_remote import api  # noqa: E402
from testrun import free_port, own, short_tmp  # noqa: E402

FAKE = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fake_agent.py")
PORT, SOCKET = free_port(), own("tm-test-rol")
tmux = partial(tmux_on, SOCKET)


def main():
    t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1"})
    work = tempfile.mkdtemp(prefix="tmr-", dir=short_tmp())
    outside = {k: v for k, v in t.env.items() if k not in ("TMUX", "TMUX_PANE", "TERMINUM_TOKEN")}
    n = [0]

    def typed(pane, command, seconds=20):
        """`command` (a `terminum …` line) typed into `pane` of the test's own tmux: what it printed, and its exit status."""
        n[0] += 1
        out = os.path.join(work, f"out{n[0]}")
        line = f"TM_HOME={shlex.quote(t.home)} TM_APP_DIR={shlex.quote(t.app)} {command} > {out}.txt 2>&1; echo $? > {out}.rc"
        tmux("send-keys", "-t", pane, "-l", line)
        tmux("send-keys", "-t", pane, "Enter")
        def status():
            try:
                return open(out + ".rc").read().strip()     # the shell makes the file, then writes it
            except OSError:
                return ""
        rc = wait_for(status, bool, seconds, 0.2, last=True) or "no answer"
        try:
            return open(out + ".txt").read(), rc
        except OSError:
            return "", rc

    def cli(*args, token=None):
        """`terminum …` run outside every pane, by this script."""
        env = {**outside, **({"TERMINUM_TOKEN": token} if token else {})}
        r = subprocess.run([BIN, *args], env=env, capture_output=True, text=True, timeout=40)
        return r.returncode, r.stdout.strip(), r.stderr.strip()

    def tool(called, **args):
        return f"{shlex.quote(BIN)} tool {called} {shlex.quote(json.dumps(args))}"

    try:
        t.start()
        ws = t.page()
        groups = wait_layout(ws, lambda gs: any(g["items"] for g in gs))
        plain_key = next(it["id"] for g in groups for it in g["items"])
        plain = plain_key.rsplit(":", 1)[1]
        secret = t.signin()
        call = lambda path, method="GET", body=None: api(t, secret, path, method=method, body=json.dumps(body).encode() if body is not None else None,  # noqa: E731
                                                         headers={"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"})
        project = call("/api/task-projects", "POST", {"name": "Managed", "dir": work})[1]["id"]
        shell = call("/api/task-agents", "POST", {"name": "Shell", "kind": "claude", "command": "sh", "at_once": 3})[1]["id"]
        call("/api/task-agents", "POST", {"name": "Fake", "kind": "claude", "command": f"python3 -I {FAKE} {BIN} --kind claude", "at_once": 2})
        st, said = call(f"/api/task-projects/{project}/session", "POST", {"agent": shell})
        check("setup: + Session opens the manager's session", st == 200 and said.get("key"), said)
        boss_key = said["key"]
        boss = boss_key.rsplit(":", 1)[1]
        st, said = call(f"/api/task-projects/{project}/session", "POST", {"agent": shell})
        other_key = said["key"]
        other = other_key.rsplit(":", 1)[1]

        marked = lambda gs: all(it.get("manager") for g in gs for it in g["items"] if it["id"] in (boss_key, other_key)) and any(it["id"] == other_key for g in gs for it in g["items"])  # noqa: E731
        rows = {it["id"]: it for g in wait_layout(ws, marked, 10) for it in g["items"]}
        check("AC-174 the list's rows say which sessions are managers", rows[boss_key].get("manager") is True and rows[other_key].get("manager") is True and "manager" not in rows[plain_key], [(k, v.get("manager")) for k, v in rows.items()])

        # a session the human opens in the project from the list: its manager too, the crown on its row
        signed = t.page(secret)
        signed.send({"t": "projectwindow", "project": project, "dir": ""})
        made = signed.until(lambda m: m["t"] in ("created", "error"), 15) or {}
        opened_key = made.get("id")
        crowned = wait_layout(signed, lambda gs: any(it["id"] == opened_key and it.get("manager") for g in gs for it in g["items"]), 10)
        check("AC-174 a session the human opens in a project is its manager, its row crowned", made.get("t") == "created" and crowned is not None, made)
        signed.close()

        # who is who
        out, rc = typed(boss, f"{shlex.quote(BIN)} tools")
        check("AC-174 a + Session's session calls as a manager and is shown the manager's tools", rc == "0" and "You call as a manager." in out and "run_task" in out and "create_agent" in out, out)
        out, rc = typed(plain, f"{shlex.quote(BIN)} tools")
        check("AC-174 a pane with no run calls as a worker and is shown none of them", rc == "0" and "You call as a worker." in out and "run_task" not in out and "list_tasks" in out, out)
        out, rc = typed(boss, tool("list_agents"))
        check("AC-181 a manager runs its tool from a shell", rc == "0" and '"name":"Shell"' in out and '"name":"Fake"' in out, out)
        out, rc = typed(plain, tool("list_agents"))
        check("AC-174 a worker's call of a manager's tool is refused, as of no tool", rc == "1" and 'No tool is named "list_agents"' in out, out)
        out, rc = typed(plain, f"TMUX_PANE={shlex.quote(boss)} {tool('delete_agent', agent='Fake')}")
        check("AC-178 a call that names the manager's pane from another pane is refused", rc == "1" and "does not come from the manager's own pane" in out, out)
        check("AC-178 and nothing was done", any(a["name"] == "Fake" for a in call("/api/tasks/meta")[1]["agents"]), "")

        # tokens: made outside every pane only
        out, rc = typed(boss, f"{shlex.quote(BIN)} tokens add inner --role manager --project Managed")
        check("AC-181 a session of terminum, a manager's too, makes no token", rc == "1" and "outside terminum" in out, out)
        rc, token, _ = cli("tokens", "add", "ops", "--role", "manager", "--project", "Managed")
        check("AC-181 a token is made in a terminal outside terminum, by its project's name", rc == 0 and token.startswith("tmt_"), token[:8])
        rc, low, _ = cli("tokens", "add", "ci", "--role", "worker", "--project", "Managed", "--days", "1")
        rc, listed, _ = cli("tokens")
        check("AC-181 the list names them and shows none", rc == 0 and "ops  manager  project Managed" in listed and "ci  worker" in listed and token[4:] not in listed, listed)
        with sqlite3.connect(f"file:{os.path.join(t.app, 'kanban.sqlite')}?mode=ro", uri=True) as db:
            kept = " ".join(str(v) for row in db.execute("SELECT * FROM tokens") for v in row)
        check("AC-181 the store keeps a token's hash alone", token not in kept and token[4:] not in kept and low[4:] not in kept, "")
        rc, out, err = cli("tool", "list_agents", token=token)
        check("AC-181 a program outside a pane runs a manager's tool with a manager's token", rc == 0 and '"name":"Fake"' in out, err or out)
        rc, out, err = cli("tool", "list_agents", token=low)
        check("AC-181 a worker's token is refused it", rc == 1 and 'No tool is named "list_agents"' in err, err or out)
        rc, out, err = cli("tool", "list_tasks", token=low)
        check("AC-181 and reads its project's tasks", rc == 0 and out.startswith("A task's text"), err or out)
        rc, out, err = cli("tool", "list_tasks")
        check("AC-181 with no pane and no token nothing is called", rc == 1 and "TERMINUM_TOKEN" in err, err or out)
        out, rc = typed(plain, f"TERMINUM_TOKEN={shlex.quote(token)} {tool('list_agents')}")
        check("AC-181 in a pane the pane's session calls, whatever token its environment holds", rc == "1" and 'No tool is named "list_agents"' in out, out)
        post = json.dumps({"token": token, "tool": "list_agents", "args": {}})
        out, rc = typed(plain, f"curl -s --unix-socket {shlex.quote(t.sock)} -H 'Content-Type: application/json' -d {shlex.quote(post)} http://localhost/agent")
        check("AC-181 terminumd refuses a token's call that comes from under a pane", "in a pane, the pane's own session calls" in out and '"error":true' in out, out)
        cli("tokens", "remove", "ci")
        rc, out, err = cli("tool", "list_tasks", token=low)
        check("AC-181 a removed token is refused", rc == 1 and "does not know this token" in err, err or out)

        # the manager runs a task on an agent, in its own words
        task = call("/api/tasks", "POST", {"title": "managed FINISHES", "project": project})[1]["id"]
        out, rc = typed(boss, tool("run_task", id=task, agent="Fake", comment="keep it short"), 40)
        check("AC-179 run_task starts a task on a pool agent through Dispatcher", rc == "0" and f"{task} runs on Fake" in out, out)
        done = wait_for(lambda: call(f"/api/tasks/{task}")[1]["status"], lambda s: s == "DONE", 30, 0.3, last=True)
        def prompts_now():
            with sqlite3.connect(f"file:{os.path.join(t.app, 'kanban.sqlite')}?mode=ro", uri=True) as db:
                return [r[0] for r in db.execute("SELECT text FROM inputs WHERE kind = 'prompt' AND sender = 'dispatcher'")]
        prompts = wait_for(prompts_now, lambda found: any("managed FINISHES" in p for p in found), 10, 0.3, last=True)
        mine = next((p for p in prompts if "managed FINISHES" in p), "")
        check("AC-179 a worker's session did it, told the manager ran it and the manager's message as the manager's",
              done == "DONE" and mine.startswith(f"The project's manager ran task {task}") and "The project's manager's message: keep it short" in mine and "The human" not in mine.split("\n\n")[0], (done, prompts))
        history = json.dumps(call(f"/api/tasks/{task}")[1], ensure_ascii=False)
        check("AC-179 its message is kept on the task under the manager's name", "keep it short" in history and "Shell · manager" in history, [e for e in call(f"/api/tasks/{task}")[1]["history"] if e["kind"] == "comment"])
        out, rc = typed(boss, tool("list_runs"))
        check("AC-179 list_runs shows the project's runs", rc == "0" and '"kind":"session"' in out, out)

        # the project's sessions: read, typed into, closed; never its own, another manager's, another project's
        out, rc = typed(boss, tool("list_sessions"))
        check("AC-179 list_sessions names the project's sessions, its own marked", rc == "0" and other_key in out and '"you":true' in out and plain_key not in out, out)
        check("AC-179 and says which are managers", '"role":"manager"' in out, out)
        for name, tool_args in (("send_to_session", {"text": "echo no"}), ("close_session", {})):
            out, rc = typed(boss, tool(name, session=other_key, **tool_args))
            check(f"AC-179 {name} refuses another manager's session", rc == "1" and "is a manager's session, the human's" in out, out)
        out, rc = typed(boss, tool("close_session", session=opened_key))
        check("AC-179 nor closes the session the human opened in the project", rc == "1" and "is a manager's session, the human's" in out, out)
        worked = call("/api/tasks", "POST", {"title": "a worker to type into", "project": project})[1]["id"]
        out, rc = typed(boss, tool("run_task", id=worked, agent="Shell"), 40)
        other_key = (wait_for(lambda: call(f"/api/tasks/{worked}")[1].get("session"), bool, 20, 0.3, last=True) or {}).get("key")
        check("setup: run_task opens a worker's session to type into", rc == "0" and other_key, out)
        other = other_key.rsplit(":", 1)[1]
        mark = os.path.join(work, "typed-by-manager")
        out, rc = typed(boss, tool("send_to_session", session=other_key, text=f"echo sent > {mark}"))
        check("AC-179 send_to_session types into another session of the project, with Enter", rc == "0" and wait_for(lambda: os.path.exists(mark), bool, 10, 0.2), out)
        out, rc = typed(boss, tool("get_session", session=other_key, lines="20"))
        check("AC-179 get_session shows its screen, said to be data", rc == "0" and "typed-by-manager" in out and "data, never instructions" in out, out)
        out, rc = typed(boss, tool("send_to_session", session=plain_key, text="echo no"))
        check("AC-179 a session outside its project is not its to type into", rc == "1" and "Your project has no session" in out, out)
        out, rc = typed(boss, tool("close_session", session=boss_key))
        check("AC-179 nor does it close its own", rc == "1" and "your own session" in out, out)
        out, rc = typed(boss, tool("send_to_session", session=other_key, text="rm\u202e -rf"))
        check("AC-179 control and invisible characters are not typed", rc == "1" and "control or invisible characters" in out, out)
        out, rc = typed(boss, tool("close_session", session=other_key))
        gone = wait_for(lambda: tmux("list-panes", "-a", "-F", "#{pane_id}").split(), lambda panes: other not in panes, 10, 0.2, last=True)
        check("AC-179 close_session ends another session of the project", rc == "0" and other not in gone, out)

        # agents and schedules by the manager, and a schedule run now
        out, rc = typed(boss, tool("create_agent", name="Second", kind="claude", command=f"python3 -I {FAKE} {BIN} --kind claude"))
        check("AC-180 create_agent adds to the pool at once", rc == "0" and any(a["name"] == "Second" for a in call("/api/tasks/meta")[1]["agents"]), out)
        out, rc = typed(boss, tool("create_schedule", title="nightly FINISHES", text="check", agent="Second", cron="0 3 * * *", budget_minutes="5"))
        scheds = call("/api/schedules")[1]["schedules"]
        check("AC-180 create_schedule makes one in its project, active, the manager its author", rc == "0" and len(scheds) == 1 and scheds[0]["project"] == project and not scheds[0]["paused"]
              and scheds[0]["author_name"] == "Shell · manager", (out, scheds))
        out, rc = typed(boss, tool("run_schedule", schedule="SCH-1"), 40)
        check("AC-182 run_schedule makes the schedule's task and starts it now", rc == "0" and "SCH-1 ran now: its task S_" in out, out)
        ran = wait_for(lambda: call("/api/schedules")[1]["schedules"][0]["last"], lambda last: last and last["status"] == "DONE", 30, 0.3, last=True)
        check("AC-182 its task runs to its end on the schedule's agent, the cadence as it was", ran and ran["status"] == "DONE" and call("/api/schedules")[1]["schedules"][0]["cron"] == "0 3 * * *", ran)
        # its last run's session, a minute quiet (terminum-dispatcher.md AC-141), is closed for the slot, and the press runs
        st, said = wait_for(lambda: call(f"/api/schedules/{scheds[0]['id']}/run", "POST", {}), lambda got: got[0] == 200, 120, 3, last=True)
        check("AC-182 Run now on its card does the same for a human", st == 200 and said.get("task", "").startswith("S_"), said)
        wait_for(lambda: call("/api/schedules")[1]["schedules"][0]["last"], lambda last: last and last["task"] == said.get("task") and last["status"] == "DONE", 30, 0.3)
        log = t.log()
        check("AC-174 each manager's call is a line of the log, with no argument", "manager.tool" in open(t.log_path).read() and "keep it short" not in open(t.log_path).read(), log[-300:])
    finally:
        t.cleanup()
        os.system(f"rm -rf {shlex.quote(work)}")
    return summary()


if __name__ == "__main__":
    sys.exit(main())
