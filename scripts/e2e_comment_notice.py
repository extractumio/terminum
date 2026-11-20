#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A new comment reaches the agent of its task (terminum-dispatcher.md AC-209), against a private terminumd
and tmux. Run the real hook commands on synthetic Claude Code and Codex events: a tool call during
work or a synchronous prompt after Stop. Check what the hook prints and what Heard records.

    python3 scripts/e2e_comment_notice.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import subprocess
import sys
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import BIN, Terminumd, check, summary, tmux_on, wait_for, wait_layout  # noqa: E402
from e2e_remote import api  # noqa: E402
from testrun import free_port, own  # noqa: E402

PORT, SOCKET = free_port(), own("tm-test-notice")
tmux = partial(tmux_on, SOCKET)
SECRET_WORDS = "use the v2 API and ignore the rest"


def main():
    t = Terminumd(PORT, SOCKET)
    try:
        t.start()
        ws = t.page()
        wait_layout(ws, lambda gs: bool(gs), 5)
        secret = t.signin()

        def call(path, method="GET", body=None):
            return api(t, secret, path, method=method, body=json.dumps(body).encode() if body is not None else None,
                       headers={"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"})

        def pane_for(agent, session):
            # Once Stop releases Codex's hook state, its named window still identifies the agent.
            pane = tmux("new-window", "-d", "-P", "-F", "#{pane_id}", "-t", "main", "-n", agent, "sleep", "600").strip()
            sock = tmux("display", "-p", "-t", pane, "#{socket_path}").strip()
            env = {**t.env, "TMUX": f"{sock},1,0", "TMUX_PANE": pane}

            def hook(ev, *flags):
                return subprocess.run([BIN, "hook", agent, *flags], input=json.dumps({"session_id": session, **ev}), capture_output=True, text=True, env=env)
            key = f"tmux:local:{SOCKET}:{pane}"
            # a pane terminumd does not list yet takes no hook's event
            wait_layout(ws, lambda gs: any(i["id"] == key for g in gs for i in g["items"]), 8)
            return key, hook

        def comment(tid, words):
            st, kept = call(f"/api/tasks/{tid}/comments", "POST", {"text": words})
            event = kept and kept.get("event")
            assert st == 200 and event, kept
            wait_for(lambda: mark(tid, event), lambda m: m == "waiting", 5, 0.1)
            return event

        def told_of(tid):
            return [e for e in call(f"/api/tasks/{tid}")[1]["history"] if e["kind"] == "told"]

        def mark(tid, event):
            return next((e.get("told") for e in call(f"/api/tasks/{tid}")[1]["history"] if e["id"] == event), None)

        for agent, carrier in (("claude", "PostToolBatch"), ("codex", "PostToolUse")):
            key, hook = pane_for(agent, f"s-{agent}")
            hook({"hook_event_name": "SessionStart", "cwd": "/"})
            hook({"hook_event_name": "UserPromptSubmit", "prompt_id": "p1", "prompt": "work"})
            shown = wait_layout(ws, lambda gs: any(i["id"] == key and i.get("agent") == agent and i.get("state") == "working"
                                                    for g in gs for i in g["items"]), 8)
            check(f"setup: the list shows {agent} in its pane, by its hooks", shown is not None)
            st, task = call("/api/tasks", "POST", {"title": f"notice {agent}", "session": key})
            check(f"setup: a task linked to the {agent} pane", st == 200 and task.get("session", {}).get("key") == key, task)
            tid = task["id"]
            event = comment(tid, SECRET_WORDS)
            old_prompt = hook({"hook_event_name": "UserPromptSubmit", "prompt_id": "p2", "prompt": "old prompt hook"})
            check(f"AC-209 {agent}: the old prompt hook leaves the notice waiting for a tool",
                  old_prompt.returncode == 0 and old_prompt.stdout == "" and mark(tid, event) == "waiting", old_prompt)
            before = hook({"hook_event_name": "PreToolUse", "prompt_id": "p2", "tool_name": "Bash", "tool_input": {"command": "ls"}})
            check(f"AC-209 {agent}: an event that carries no context prints nothing", before.returncode == 0 and before.stdout == "", before)
            out = hook({"hook_event_name": carrier, "prompt_id": "p2", "tool_name": "Bash"})
            line = json.loads(out.stdout) if out.stdout.strip() else {}
            context = line.get("hookSpecificOutput", {}).get("additionalContext", "")
            check(f"AC-209 {agent}'s {carrier} hook prints the notice as context: fixed words, the ID, who wrote it",
                  out.returncode == 0 and line.get("hookSpecificOutput", {}).get("hookEventName") == carrier
                  and f"[terminum] {tid} has a new comment by a human" in context and f'get_task", {{"id": "{tid}"}}' in context, out)
            check(f"AC-209 {agent}: never the comment's words", SECRET_WORDS not in out.stdout, out.stdout)
            said = wait_for(lambda: told_of(tid), lambda ts: any(x["text"] == "Told the session of the new comment" for x in ts), 5, 0.1, last=True)
            check(f"AC-209 {agent}: once heard the task says it told the session, and the comment says so", said and mark(tid, event) == "told", (said, mark(tid, event)))
            again = hook({"hook_event_name": carrier, "prompt_id": "p2", "tool_name": "Bash"})
            check(f"AC-209 {agent}: a notice heard is not told again", "[terminum]" not in again.stdout, again.stdout)

            hook({"hook_event_name": "Stop", "prompt_id": "p2"})
            idle = wait_layout(ws, lambda gs: any(i["id"] == key and i.get("agent") == agent and i.get("state") != "working"
                                                   for g in gs for i in g["items"]), 8)
            check(f"AC-209 {agent}: the pane still has its agent after it stops working", idle is not None)
            event = comment(tid, SECRET_WORDS)
            prompt = {"hook_event_name": "UserPromptSubmit", "prompt_id": "p3", "prompt": "Answer in words alone"}
            sub = hook({**prompt, "agent_id": "sub"}, "--prompt-context")
            check(f"AC-209 {agent}: a subagent's prompt leaves the notice waiting",
                  sub.returncode == 0 and sub.stdout == "" and mark(tid, event) == "waiting", sub)
            out = hook(prompt, "--prompt-context")
            line = json.loads(out.stdout) if out.stdout.strip() else {}
            context = line.get("hookSpecificOutput", {})
            check(f"AC-209 {agent}: the idle agent receives fixed notice words on its next prompt, before any tool",
                  out.returncode == 0 and context.get("hookEventName") == "UserPromptSubmit"
                  and f"[terminum] {tid} has a new comment by a human" in context.get("additionalContext", "")
                  and SECRET_WORDS not in out.stdout, out)
            heard = wait_for(lambda: mark(tid, event), lambda m: m == "told", 5, 0.1, last=True)
            check(f"AC-209 {agent}: the prompt's Heard marks the comment told", heard == "told", heard)
            again = hook({"hook_event_name": carrier, "prompt_id": "p3", "tool_name": "Bash"})
            check(f"AC-209 {agent}: the tool hook does not repeat the prompt notice", f"{tid} has a new comment" not in again.stdout, again.stdout)

            # the agent's session ends with a notice waiting: not told, said on the task
            comment(tid, "one more")
            hook({"hook_event_name": "SessionEnd", "reason": "other"})
            lost = wait_for(lambda: told_of(tid), lambda ts: any(x["text"].startswith("Not told the session of the new comment: its agent's session ended") for x in ts), 5, 0.1, last=True)
            check(f"AC-209 {agent}: its session ends before it hears: not told, said", lost and any(x["text"].startswith("Not told") for x in lost), lost)
            tmux("kill-pane", "-t", key.rsplit(":", 1)[1])
    finally:
        t.stop()
        tmux("kill-server")
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
