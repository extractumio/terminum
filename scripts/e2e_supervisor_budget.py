#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-237 against a private terminumd: a real round spends the judgment ceiling; then investigations wait while
collection, the critical line and the digest go on, a due weekly review becomes the owner's budget card, and the
review run after the owner raises the ceiling lists that card among the decisions waiting."""
from contextlib import closing
import json
from pathlib import Path
import shlex
import sqlite3
import time

from e2e_remote import api
from e2e_ws import BIN, Terminumd, check, summary, unix_request, wait_for
from supervisor_report_checks import critical_report
from testrun import free_port, own

FAKE = Path(__file__).resolve().with_name("fake_agent.py")
PORT, SOCKET = free_port(), own("tm-test-sbudget")
CEILING = 1000
NOTICE = "Supervisor judgment budget limit reached. Code-only work continues."
CARD = "Supervisor review needs a budget decision"
WEEKLY = "0 9 * * 1"


def main():
    t = Terminumd(PORT, SOCKET, env={"TM_TEST_HOOKS": "1", "TM_TEST_CLOCK_SCALE": "60"})
    prompts, log, notices = (Path(t.root, name) for name in ("budget-prompts", "budget-log", "budget-line"))
    headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"}
    secret, page = "", None

    def db(statement, bindings=()):
        with closing(sqlite3.connect(Path(t.app, "kanban.sqlite"))) as conn, conn:
            return conn.execute(statement, bindings).fetchall()

    def request(path, method="GET", body=None):
        return api(t, secret, path, method=method, body=json.dumps(body).encode() if body is not None else None, headers=headers)

    def call(path, method="GET", body=None):
        status, reply = request(path, method, body)
        assert status == 200, (path, status, reply)
        return reply

    def logged():
        return [json.loads(line) for line in log.read_text().splitlines()] if log.exists() else []

    def sent():
        return notices.read_text() if notices.exists() else ""

    def failed_tasks(count):
        for _ in range(count):
            task = call("/api/tasks", "POST", {"title": "Budget fixture", "text": "Fixture task body.", "project": project})["id"]
            call(f"/api/tasks/{task}", "PATCH", {"status": "FAILED"})
            call(f"/api/tasks/{task}/comments", "POST", {"text": "Fixture comment used as data."})

    def signal(detector, metric=None):
        rows = db("SELECT fingerprint, count, state, first_seen FROM supervisor_signals WHERE project = ? AND detector = ?"
                  " AND (? IS NULL OR metric = ?)", (project, detector, metric, metric))
        return rows[0] if rows else None

    def raise_stakes(key):
        # The collector supplied the signal and its evidence. A drill raises only its stakes, to level 2.
        db("UPDATE supervisor_signals SET scores = json_set(scores, '$.blast', 2, '$.confidence', 0.9) WHERE fingerprint = ?", [key])

    def judgment():
        return call(path + "/screen")["spending"]["judgment"]

    def edit_schedule(schedule, cron):
        fields = ("title", "body", "project", "severity", "agent", "once_at", "paused")
        call(f"/api/schedules/{schedule['id']}", "PATCH", {**{k: schedule[k] for k in fields}, "cron": cron,
                                                           "budget_minutes": schedule["budget_seconds"] // 60})

    try:
        t.start()
        secret = t.signin()
        project = call("/api/task-projects", "POST", {"name": "Budget", "dir": t.root})["id"]
        path = f"/api/task-projects/{project}/supervisor"
        command = ["python3", "-I", str(FAKE), BIN, "--prompts", str(prompts), "--round-log", str(log),
                   "--round-mode", "finding", "--round-tokens", str(CEILING)]
        agent = call("/api/task-agents", "POST", {"project": project, "name": "Budget agent", "kind": "claude",
                                                  "at_once": 2, "command": shlex.join(command)})["id"]
        channel = Path(t.root, "line channel")
        channel.write_text("#!/bin/sh\ncat >> " + shlex.quote(str(notices)) + "\n")
        channel.chmod(0o700)
        call(path + "/line", "PUT", {"revision": 0, "settings": dict(
            channels=[str(channel)], link_base="https://terminum.example", quiet=None, ladder=[10, 30, 60])})
        budgets = dict(judgment_floor_tokens=CEILING, judgment_ceiling=CEILING, round_token_ceiling=CEILING)
        call(path + "/settings", "PUT", {"revision": 0, "settings": {
            "level": "advisor", "tiers": dict(strong=agent, medium=agent, small=agent), "budgets": budgets}})
        failed_tasks(4)
        call(path, "PUT", {"enabled": True})
        [schedule] = [s for s in call("/api/schedules")["schedules"] if s["project"] == project]
        key = wait_for(lambda: signal("flow.failed_no_reason"), bool, 20, every=0.2)[0]
        raise_stakes(key)

        rounds = lambda: db("SELECT id, run, ended_at, failure, finding FROM supervisor_rounds WHERE project = ?", [project])
        [(case, run, _, failure, finding)] = wait_for(rounds, lambda r: len(r) == 1 and r[0][2], 30, every=0.2)
        spent = wait_for(lambda: db("SELECT coalesce(sum(input + output + cache_write + cache_read), 0) FROM usage WHERE run = ?",
                                    [run])[0][0], lambda n: n == CEILING, 15, every=0.2)
        stopped = wait_for(judgment, lambda j: j["investigation_left"] == 0, 15, every=0.2)
        check("AC-237 a real round's reported usage reaches the judgment ceiling within its own cap",
              spent == CEILING and failure is None and finding is not None
              and call(f"{path}/rounds/{case}")["case"]["budget_tokens"] == CEILING
              and stopped["tokens"] == stopped["limit_tokens"] == CEILING and stopped["routine_left"] == 0,
              (spent, failure, stopped))
        raised = wait_for(lambda: signal("self.budget", "judgment_tokens"), lambda s: s and s[2] == "raised", 20, every=0.2, last=True)
        check("AC-237 code raises self.budget once the ceiling is reached", bool(raised) and raised[2] == "raised", raised)
        exhausted_at = raised[3] if raised else time.time()

        status, refused = request(f"{path}/signals/{key}/investigate", "POST", {})
        check("AC-237 the owner's investigate press waits on the budget notice", status != 200 and NOTICE in refused.get("error", ""),
              (status, refused))
        before = signal("flow.failed_no_reason")[1]
        beat = call(path)["last_ok"]
        failed_tasks(2)
        grown = wait_for(lambda: signal("flow.failed_no_reason"), lambda s: s[1] > before, 30, every=0.2)
        raise_stakes(key)   # the new evidence rewrote its scores
        reason = wait_for(lambda: db("SELECT reason FROM supervisor_round_requests WHERE fingerprint = ?", [key]),
                          lambda r: r == [(NOTICE,)], 20, every=0.2)
        check("AC-237 collection goes on and the new evidence's investigation waits with the budget notice",
              grown[1] > before and call(path)["last_ok"] > beat and reason == [(NOTICE,)] and len(rounds()) == 1,
              (grown, reason, rounds()))

        page = t.page(secret)
        page.send({"t": "projectwindow", "project": project, "dir": ""})
        pane = page.until(lambda m: m.get("t") == "created", 15)["id"].rsplit(":", 1)[1]
        for event in ["SessionStart", "PreToolUse", "PreToolUse", "PreToolUse", "PreToolUse"]:
            status, _ = unix_request(t.sock, "POST", "/hook", {"agent": "claude", "session": "budget-session", "event": event,
                                                              "socket": SOCKET, "pane": pane, "tool_call_hash": "abcdef1234567890"})
            assert status in (200, 204), status
        loop = wait_for(lambda: signal("drift.loop"), bool, 30, every=0.2)
        check("AC-237 a new deterministic signal is still collected", loop is not None and loop[3] >= exhausted_at, loop)

        report, out, rc = critical_report(t, SOCKET, pane)
        incident = wait_for(lambda: next((i for i in call("/api/supervisor/incidents")
                                          if report and report in i.get("reports", [])), None), bool, 15, every=0.2)
        page_text = f"{NOTICE}\nCritical incident {incident['id']} · Level 3." if incident else "no incident"
        check("AC-237 a critical incident still pages, and the owner's message starts with the budget notice",
              page_text in wait_for(sent, lambda text: page_text in text, 20, every=0.2, last=True), (out, rc, sent()))

        # The daily digest already went out today: the drill moves it back a day, as if the next day had come.
        db("UPDATE supervisor_digests SET at = at - 86400 WHERE project = ?", [project])
        critical = signal("reports.critical")
        digest = wait_for(lambda: call(path + "/screen")["digest"], lambda d: d and d["at"] >= exhausted_at, 30, every=0.2, last=True)
        check("AC-237 the exception digest is still published without a round, with a signal first seen after the ceiling",
              bool(digest and critical and critical[3] >= exhausted_at and digest["last_ok"])
              and any(e["fingerprint"] == critical[0] for e in digest["entries"]) and len(rounds()) == 1, (critical, digest))

        status, pressed = request(f"/api/schedules/{schedule['id']}/run", "POST", {})
        check("AC-237 the owner's press of the weekly review waits on the budget notice",
              status != 200 and NOTICE in pressed.get("error", ""), (status, pressed))
        edit_schedule(schedule, "* * * * *")
        # A save starts the cadence at its own time: set back two minutes, this minute's firing is due now, not late.
        db("UPDATE schedules SET last_due = last_due - 120 WHERE id = ?", [schedule["id"]])
        cards = lambda: db("SELECT t.slug, t.status FROM supervisor_budget_cards c JOIN tasks t ON t.id = c.task"
                           " WHERE c.project = ? AND c.kind = 'judgment' AND t.title = ?", (project, CARD))
        card = wait_for(cards, bool, 20, every=0.2)
        skipped = db("SELECT why FROM schedule_fires WHERE schedule = ? AND state = 'skipped'", [schedule["id"]])
        check("AC-237 the due weekly review that cannot run becomes one decision card, and no review starts",
              len(card) == 1 and card[0][1] == "WAIT FOR HUMAN" and (NOTICE,) in skipped
              and not any("review" in entry for entry in logged()), (card, skipped))
        edit_schedule(schedule, WEEKLY)

        saved = call(path + "/settings")
        saved["settings"]["budgets"].update(judgment_floor_tokens=400000, judgment_ceiling=400000, round_token_ceiling=100000)
        call(path + "/settings", "PUT", {"revision": saved["revision"], "settings": saved["settings"]})
        wait_for(lambda: signal("self.budget", "judgment_tokens"), lambda s: s[2] == "resolved", 20, every=0.2)
        task = call(f"/api/schedules/{schedule['id']}/run", "POST", {})["task"]
        review = wait_for(lambda: call(path + "/screen")["review"], lambda r: r and r["task"] == task, 30, every=0.2)
        waiting = review["result"].split("## Top three register problems")[0] if review else ""
        check("AC-237 once the owner raises the ceiling, the weekly review lists the budget card among decisions waiting",
              f"{card[0][0]} · {CARD}" in waiting and NOTICE not in waiting, waiting)
    except Exception:
        print(t.log())
        raise
    finally:
        if page:
            page.close()
        t.cleanup()


if __name__ == "__main__":
    main()
    summary()
