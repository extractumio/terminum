#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Coding agents' hooks (terminum-terminal.md AC-39) through `terminum hook`, as an agent runs it
in its pane, against a private terminumd and a private tmux server (never the user's): states,
tools, questions and failures in the list over the screen's reading; no output; its time.

    python3 scripts/e2e_hooks.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import statistics
import subprocess
import sys
import time
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import BIN, Terminumd, check, summary, tmux_on, wait_layout  # noqa: E402

PORT, SOCKET = 47975, "tm-test-hooks"
tmux = partial(tmux_on, SOCKET)


def main():
    t = Terminumd(PORT, SOCKET)
    try:
        t.start()
        ws = t.page()
        last = [[]]

        def row(pane):
            return next((i for g in last[0] for i in g["items"] if i["id"].endswith(":" + pane)), None)

        def seen(pane, pred, timeout=8):
            def keep(gs):
                last[0] = gs
                return (r := row(pane)) is not None and pred(r)
            if (r := row(pane)) is not None and pred(r):
                return r
            return row(pane) if wait_layout(ws, keep, timeout) else (print("  last row:", row(pane)) or None)

        wait_layout(ws, lambda gs: bool(gs), 5)
        pane = tmux("new-window", "-d", "-P", "-F", "#{pane_id}", "-t", "main", "sleep", "600").strip()
        sock = tmux("display", "-p", "-t", pane, "#{socket_path}").strip()
        env = {**t.env, "TMUX": f"{sock},1,0", "TMUX_PANE": pane}
        took = []

        def hook(agent, ev, pane_env=None):
            start = time.monotonic()
            p = subprocess.run([BIN, "hook", agent], input=json.dumps(ev), capture_output=True, text=True, env={**env, **(pane_env or {})})
            took.append(time.monotonic() - start)
            return p

        first = seen(pane, lambda r: True)
        check("AC-39 a pane with no hooks and no agent on its screen shows no state", first and first["state"] is None, first)
        p = hook("claude", {"hook_event_name": "UserPromptSubmit", "session_id": "s1", "prompt_id": "p1", "prompt": "run the tests"})
        check("AC-39 the hook prints nothing and exits 0", p.returncode == 0 and p.stdout == "" and p.stderr == "", p)
        hook("claude", {"hook_event_name": "PreToolUse", "session_id": "s1", "prompt_id": "p1", "tool_name": "Bash",
                        "tool_input": {"command": "npm test --token=abc"}, "transcript_path": "/x"})
        r = seen(pane, lambda r: r["state"] == "working" and r.get("tool"))
        check("AC-39 hooks say working and the tool, on a screen that shows no agent", r and r["agent"] == "claude" and r["tool"] == "Bash: npm test", r)
        hook("claude", {"hook_event_name": "SubagentStart", "session_id": "s1", "prompt_id": "p1", "agent_id": "a1"})
        r = seen(pane, lambda r: r.get("subagents") == 1)
        check("AC-39 a subagent counted", r is not None, r)
        hook("claude", {"hook_event_name": "PermissionRequest", "session_id": "s1", "tool_name": "Bash", "tool_input": {"command": "rm -rf build"}})
        r = seen(pane, lambda r: r["state"] == "waiting")
        check("AC-39 an approval waits and says for what", r and r.get("why") == "Approve Bash: rm -rf", r)
        hook("claude", {"hook_event_name": "StopFailure", "session_id": "s1", "prompt_id": "p1", "error": "rate_limit"})
        r = seen(pane, lambda r: r["state"] == "failed")
        check("AC-39 a failed turn says why", r and r.get("why") == "rate_limit" and "subagents" not in r and "tool" not in r, r)
        hook("claude", {"hook_event_name": "UserPromptSubmit", "session_id": "s1", "prompt_id": "p2"})
        hook("claude", {"hook_event_name": "Stop", "session_id": "s1", "prompt_id": "p2"})
        r = seen(pane, lambda r: r["state"] == "done")
        check("AC-39 a stop is done", r is not None, r)
        p = hook("claude", {"hook_event_name": "Stop"}, {"TMUX_PANE": "%999"})
        p2 = hook("claude", {"hook_event_name": "Stop"}, {"TMUX": "/private/tmp/tmux-1/other,1,0"})
        check("AC-39 a pane of no known server is ignored, quietly", p.returncode == 0 == p2.returncode and "panicked" not in t.log(), t.log()[-300:])
        # sent history (AC-90): the hook's prompt, then a page's shell command, per folder
        cwd = tmux("display", "-p", "-t", pane, "#{pane_current_path}").strip()
        hook("claude", {"hook_event_name": "UserPromptSubmit", "session_id": "s1", "prompt_id": "p3", "cwd": cwd,
                        "prompt": "explain **this**\nexport API_TOKEN=abc123"})
        ws.until(lambda m: m["t"] == "sentnew", 5)        # kept after the hook was answered (it never waits for the disk)
        ws.send({"t": "sub", "id": f"tmux:local:{SOCKET}:{pane}"})
        ws.send({"t": "sentget", "n": 0})
        one = ws.until(lambda m: m["t"] == "sentone", 5)
        check("AC-90 a hook's prompt is kept, its secret's value left out", one and one["item"] and one["item"]["text"] == "explain **this**\nexport API_TOKEN=[redacted]"
              and one["item"]["agent"] == "claude" and one["path"] == cwd, one)
        ws.send({"t": "sent", "text": "make test", "agent": "claude"})
        ws.send({"t": "sent", "text": " secret", "agent": ""})
        ws.send({"t": "sent", "text": "ls -la", "agent": ""})
        new = ws.until(lambda m: m["t"] == "sentnew", 5)
        ws.send({"t": "sentget", "n": 0})
        one = ws.until(lambda m: m["t"] == "sentone", 5)
        check("AC-90 a shell's command from the page; an agent's with hooks and one after a space are not", new and one and one["total"] == 2
              and one["item"]["text"] == "ls -la" and "agent" not in one["item"], one)
        ws.send({"t": "sentget", "n": 1})
        older = ws.until(lambda m: m["t"] == "sentone", 5)
        check("AC-90 the one before", older and older["n"] == 1 and older["item"]["agent"] == "claude", older)
        files = [os.path.join(dp, f) for dp, _, fs in os.walk(os.path.join(t.home, "sessions")) for f in fs]
        check("AC-90 kept in TM_HOME/sessions/<project>/, 0600", len(files) == 2 and all(os.stat(f).st_mode & 0o077 == 0 for f in files), files)
        # a reminder (AC-91): up when the pane's turn ends, put down by Close, up again at the next
        ws.send({"t": "mark", "ref": "r1", "sid": f"tmux:local:{SOCKET}:{pane}", "n": 0, "text": "check the retry", "note": "", "remind": True})
        made = ws.until(lambda m: m["t"] == "marked" and m.get("ref") == "r1", 5)
        rid = made and made.get("id")

        def turn(n):
            hook("claude", {"hook_event_name": "UserPromptSubmit", "session_id": "s1", "prompt_id": f"t{n}"})
            seen(pane, lambda r: r["state"] == "working")
            hook("claude", {"hook_event_name": "Stop", "session_id": "s1", "prompt_id": f"t{n}"})
            return ws.until(lambda m: m["t"] == "mark" and m["item"]["id"] == rid and m["item"].get("due"), 8)
        up = turn(1)
        check("AC-91 a reminder comes up when its pane's turn ends", rid and up is not None, made)
        ws.send({"t": "remind", "id": rid, "action": "close"})
        down = ws.until(lambda m: m["t"] == "mark" and m["item"]["id"] == rid, 5)
        again = turn(2)
        check("AC-91 closed, it comes up again at the next turn's end", down and not down["item"].get("due") and again is not None, down)
        ws.send({"t": "remind", "id": rid, "action": "resolve"})
        ws.until(lambda m: m["t"] == "mark" and m["item"]["id"] == rid, 5)
        check("AC-91 resolved, it comes up no more", turn(3) is None)
        h = t.health()
        check("AC-39 /health says when Claude Code's hooks last spoke", isinstance(h.get("hooks", {}).get("claude"), int), h.get("hooks"))
        stopped = Terminumd.__new__(Terminumd)
        stopped.env = {**env, "TM_APP_DIR": t.app + "-none"}
        os.makedirs(stopped.env["TM_APP_DIR"], mode=0o700, exist_ok=True)
        start = time.monotonic()
        p = subprocess.run([BIN, "hook", "claude"], input='{"hook_event_name":"Stop"}', capture_output=True, text=True, env=stopped.env)
        check("AC-39 without terminumd the hook returns at once", p.returncode == 0 and time.monotonic() - start < 0.5, time.monotonic() - start)
        med = statistics.median(took)
        check("AC-39 a hook takes under 50 ms (median of its runs)", med < 0.05, f"{med * 1000:.1f} ms of {len(took)}")
    finally:
        t.stop()
        tmux("kill-server")
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
