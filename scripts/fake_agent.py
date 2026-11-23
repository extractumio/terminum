#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A coding agent as Dispatcher sees it, for web/test/e2e_livedispatcher.mjs: the measured input box of
Claude Code or Codex (terminum-dispatcher.md AS-125), hooks by `terminum hook`, finish_task by `terminum
mcp`; no model. What it does with a prompt is in the prompt's words:
FINISHES, WORKS, SUBAGENT (can accompany a wait), TOKENS, ASKS, QUESTIONS (ask_human),
DEPENDENCY (wait_for_task on --wait-on), LIMIT, DIES, CLEAN, SILENT, STUCK (of a main
task with its subtasks, terminum-dispatcher.md AC-185: the last subtask and the main task fail, the rest is finished); Auto-Dispatcher's
round (terminum-dispatcher.md AC-170) starts its first candidates, one for each free slot, and finishes; a task a round started
is worked on; any other stops at once. `--stubborn`: SIGTERM is ignored. `--start trust`: a question before its box; `--start
quits`: ends after receiving its first paste, without echoing or taking it; `--start
none`: never a box; `--start fast`: exits at once with 7; `--start slow`: its first paste shows 3 s late, as a busy
machine's does, and a prompt pasted twice is not worked on. `--title`: an idle OSC title after Stop to
identify the stand-in after terminumd restarts; `--prompts <file>`: each prompt it takes, a line; `--stop-after <file>`: hold Stop until the file exists and write
<file>.sent after attempting the hook, so the test can place it during downtime."""
import json
import os
import re
import signal
import subprocess
import sys
import termios
import time
import tty

RULE = "\x1b[37m" + "─" * 60 + "\x1b[0m"
# Longer than this, what is in the box shows as a mark, not its text.
FOLD = 800
# a task's ID as a prompt names it; a schedule's task says so first (terminum-schedules.md AC-162)
TASK_ID = r"(?:S_)?[A-Z][A-Z0-9]*-\d+"


def main():
    args = sys.argv[1:]
    cli = args[0]
    opt = lambda name, default: args[args.index(name) + 1] if name in args else default
    kind, start, session = opt("--kind", "claude"), opt("--start", ""), opt("--resume", f"fake-{os.getpid()}")
    typed, pasting, pastes, late = "", False, 0, start == "slow"

    def hook(event, **more):
        # AC-131: hold Stop until the live test has stopped terminumd, then acknowledge the attempt.
        barrier = opt("--stop-after", "") if event == "Stop" else ""
        if barrier:
            while not os.path.exists(barrier):
                time.sleep(0.05)
        checked = subprocess.run([cli, "hook", kind], input=json.dumps({"session_id": session, "cwd": os.getcwd(), "hook_event_name": event, **more}), text=True, capture_output=True, timeout=8)
        if event == "Stop" and (title := opt("--title", "")):
            sys.stdout.write(f"\x1b]0;{title}\x07")
            sys.stdout.flush()
        if barrier:
            with open(barrier + ".sent", "w", encoding="utf8"):
                pass
        return checked.stdout

    def draw(body):
        sys.stdout.write("\x1b[2J\x1b[H" + "\r\n".join(body) + "\r\n")
        sys.stdout.flush()

    def box(background=""):
        # a long paste folds to a mark, as both agents' boxes do (the gate reads "[Pasted")
        held = typed if len(typed) <= FOLD else (f"[Pasted Content {len(typed)} chars]" if kind == "codex" else f"[Pasted text #1 +{len(typed)} chars]")
        if kind == "codex":
            shown = f"\x1b[1m›\x1b[0m \x1b[2mAsk Codex to do anything\x1b[0m" if not typed else f"\x1b[1m›\x1b[0m {held}"
            draw(["  OpenAI Codex", "", shown, "", "  ? for shortcuts"])
        else:
            draw(["  Claude Code", "", RULE, f"\x1b[39m❯ {held.replace(chr(10), ' ')}\x1b[7m \x1b[0m", RULE, background or "  ? for shortcuts"])

    def tool(name, arguments):
        rpc = lambda i, m, p=None: json.dumps({"jsonrpc": "2.0", **({} if i is None else {"id": i}), "method": m, **({"params": p} if p else {})})
        lines = [rpc(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "fake", "version": "1"}}), rpc(None, "notifications/initialized"),
                 rpc(2, "tools/call", {"name": name, "arguments": arguments})]
        done = subprocess.run([cli, "mcp"], input="\n".join(lines) + "\n", text=True, capture_output=True, timeout=8)
        for reply in map(json.loads, done.stdout.splitlines()):
            if reply.get("id") == 2:
                return reply.get("result", reply.get("error", {}))
        return {}

    def investigate(case_id):
        def call(name, **arguments):
            result = tool("execute_tool", {"name": name, "arguments": arguments})
            with open(opt("--round-log", ""), "a", encoding="utf8") as log:
                log.write(json.dumps({"case": case_id, "tool": name, **result}) + "\n")
            return result

        def decoded(result):
            return json.loads(result["content"][0]["text"])

        call("get_case_file", id="case_foreign")
        case = decoded(call("get_case_file", id=case_id))
        if case["kind"] == "design_review":
            call("record_design_review", id=case["task"], role=case["role"], verdict="approved",
                 compatibility="Existing clients still work.", rejected_advice="Remove the route | Clients still use it.")
            return
        if case["kind"] == "reflection":
            call("hold", signal="fixture", kind="pause_dispatch", reason="Reflection fixture.")
            call("run_task", id="TERM-1", agent="Fixture")
            change = {"detector": "flow.failed_repeat", "action": {"operation": "owner_review", "target": "charter"}}
            if case["subject_id"] == "config-tune":
                change["action"] = {"operation": "tune", "threshold": {"absolute": 2.5}}
            preview = decoded(call("replay_config_change", **change))
            adopted = decoded(call("propose_config_change", change=change, replay=preview["replay"]))
            report = dict(missed="No earlier evidence was recorded.", noise="One repeated rule fired.",
                          wrong="The timeline shows the recorded trigger.",
                          proposals=[{"change_id": adopted["change"], "evidence": [1]}])
            call("record_reflection", **{**report, "proposals": [{**report["proposals"][0], "evidence": [9999]}]})
            if "alert_wrong" in case["triggers"]:
                measured = next(row for row in case["agent_rates"] if row["task_type"] == "Bug")
                report["routing"] = {"agent": measured["agent"], "task_type": "Bug",
                                     "change": "Review the Bug default agent.", "why": "The sample is incomplete."}
            call("record_reflection", **report)
            return
        if opt("--round-mode", "") == "thread":
            barrier = opt("--thread-barrier", "")
            deadline = time.monotonic() + 12
            while not os.path.exists(barrier) and time.monotonic() < deadline:
                time.sleep(.05)
            until = time.monotonic() + 5
            while time.monotonic() < until:
                notice = hook("PostToolBatch", tool_name="Read")
                if "new comment" in notice:
                    with open(opt("--round-log", ""), "a", encoding="utf8") as log:
                        log.write(json.dumps({"notice": notice}) + "\n")
                    break
                time.sleep(.15)
            call("post_incident_answer", text="The checked fixture explains the incident.")
            return
        if opt("--round-mode", "") != "finding":
            return
        sig = case["signal"]
        query, fingerprint = sig["detector"], sig["fingerprint"]
        call("list_signals")
        call("get_signal", id=fingerprint)
        value = decoded(call("get_metric", query=query, signal=fingerprint))["measure"]["value"]
        refs = sig["evidence"]
        finding = dict(cause="A blocked dependency holds the queue.", confidence=0.85,
                       disproof="The wait continues after the dependency clears.",
                       fix_proposal="Resolve the dependency.", evidence=refs,
                       measurements=[dict(query=query, signal=fingerprint, value=value)])
        wrong = [dict(query=query, signal=fingerprint, value=value + 5)]
        call("record_finding", **{**finding, "measurements": wrong})
        call("record_finding", **{**finding, "evidence": [{"table": "events", "id": "missing"}]})
        if foreign := opt("--foreign-evidence", ""):
            call("record_finding", **{**finding, "evidence": [{"table": "tasks", "id": foreign}]})
        call("record_finding", **finding)

    def review(task):
        call = lambda name, args: tool("execute_tool", {"name": name, "arguments": args})
        context = call("get_supervisor_review", {})
        refused = call("add_comment", {"id": task, "text": "Review fixture cannot post."})
        sections = ["Decisions waiting for you", "Top three register problems", "Resolved and recurred",
                    "Effort allocation", "Agent performance", "Owner bottlenecks", "Indicator verdicts",
                    "Supervisor outcomes and budgets", "Verdict batch"]
        result = call("finish_task", {"id": task, "result": "\n".join(f"{s}: Measured review fixture." for s in sections)})
        with open(opt("--round-log", ""), "a", encoding="utf8") as log:
            log.write(json.dumps({"review": context, "refused": refused, "finished": result}) + "\n")

    def audit(task):
        call = lambda name, args: tool("execute_tool", {"name": name, "arguments": args})
        pages = {name: call("get_supervisor_audit", {"section": name})
                 for name in ("ledger", "signals", "misses", "register")}
        denied = call("get_supervisor_review", {})
        report = "  Missed issues\n\n  Noise and unsupported conclusions.\n"
        result = call("finish_task", {"id": task, "result": report})
        with open(opt("--round-log", ""), "a", encoding="utf8") as log:
            log.write(json.dumps({"audit": pages, "denied": denied, "finished": result, "report": report}) + "\n")

    def talk(case_id):
        response = tool("execute_tool", {"name": "get_case_file", "arguments": {"id": case_id}})
        with open(opt("--round-log", ""), "a", encoding="utf8") as log:
            log.write(json.dumps({"talk": case_id, **response}) + "\n")

    # a result is kept only in its sections (AC-123): the close-out's and a schedule's, both there, as
    # terminumd/src/tasks/closeout.rs names them (SECTIONS, SCHEDULED)
    SECTIONS = ["Outcome", "Done", "Checked", "Not done", "Needs a human", "New tasks", "Tasks changed", "Issues and discoveries",
                "Further recommendations", "Summary", "Details", "For the human"]

    def finish(task):
        tool("finish_task", {"id": task, "result": "\n".join(f"{s}: {'All done.' if s == 'Outcome' else 'None'}" for s in SECTIONS)})

    if "--stubborn" in args:
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
    if start == "fast":
        sys.exit(7)
    if start == "none":
        draw(["something it does not know"])
        subprocess.run(["sleep", "600"])
        return
    fd = sys.stdin.fileno()
    saved = termios.tcgetattr(fd)
    tty.setraw(fd)
    sys.stdout.write("\x1b[?2004h")
    try:
        if start == "trust":
            draw([" Accessing workspace:", "", " Quick safety check", "", " ❯ No, exit", "   Yes, I trust this folder", "", " Enter to confirm · Esc to cancel"])
            os.read(fd, 16)
        box()
        if log := opt("--round-log", ""):
            probe = subprocess.run([cli, "tools"], text=True, capture_output=True, timeout=8)
            with open(log, "a", encoding="utf8") as f:
                f.write(json.dumps({"probe": probe.stdout, "rc": probe.returncode}) + "\n")
        if kind == "claude":
            hook("SessionStart", source="resume" if "--resume" in args else "startup")
        if start == "quits":
            got = b""                 # the whole paste, however many reads it takes: nothing of it is left to the shell
            while b"\x1b[201~" not in got and (more := os.read(fd, 65536)):
                got += more
            return                    # its session opened, but it never took the task (an update's restart)
        while True:
            data = os.read(fd, 65536).decode("utf8", "replace")
            if not data:
                return
            # what is pasted (between the markers) is text, line ends too; a bare CR is Enter; a long
            # paste comes in several reads, so being inside one carries over to the next read
            ends = False
            for part in re.split(r"(\x1b\[20[01]~)", data):
                if part in ("\x1b[200~", "\x1b[201~"):
                    pasting = part == "\x1b[200~"
                    pastes += pasting
                elif pasting:
                    typed += part.replace("\r", " ").replace("\n", " ")
                else:
                    ends = ends or "\r" in part
                    if "\x15" in part:                # Ctrl+U clears the box, as both agents' do
                        typed, part = "", part.rsplit("\x15", 1)[1]
                    typed += part.split("\r")[0]
            if not ends:
                if late and typed:
                    late = False
                    time.sleep(3)             # what comes meanwhile is read after it
                box()
                continue
            prompt, typed = typed, ""
            twice, pastes = start == "slow" and pastes > 1, 0
            box()
            if "SILENT" in prompt or twice:
                continue
            if kind == "codex":
                hook("SessionStart", source="startup")
            hook("UserPromptSubmit", prompt=prompt)
            if log := opt("--prompts", ""):    # what it was typed, for a test to read whole
                with open(log, "a", encoding="utf8") as f:
                    f.write(prompt + "\n")
            several = re.search(rf"tasks: ((?:{TASK_ID}(?:, )?)+)", prompt)   # a press of several names them all (AC-138)
            tasks = re.findall(TASK_ID, several[1]) if several else re.findall(TASK_ID, prompt)[:1] or [""]
            # a main task with its subtasks: they go first, it goes last (AC-185)
            main = re.search(rf"main task ({TASK_ID}) with its (?:\d+ subtasks|subtask) still to do: ((?:{TASK_ID}(?:, )?)+)", prompt)
            if main:
                tasks = re.findall(TASK_ID, main[2]) + [main[1]]
            round_of = re.search(rf"^\[terminum Auto-Dispatcher\] Task ({TASK_ID}):.*Free agent slots: (\d+)\. Candidates, this project's TODO tasks: ((?:{TASK_ID}(?:, )?)+)", prompt, re.S)
            if "SUBAGENT" in prompt:
                hook("SubagentStart", agent_id="sub-1", agent_type="reviewer")
                if "QUESTIONS" in prompt or "DEPENDENCY" in prompt:
                    typed = "draft during wait"
                    box()
            if opt("--round-log", "") and (case := re.fullmatch(r"Supervisor (?:reflection |design review )?case (case_[a-f0-9]+)\..*", prompt)):
                investigate(case[1])
            elif opt("--round-log", "") and (case := re.fullmatch(r"Supervisor incident case (case_[a-f0-9]+)\..*", prompt)):
                talk(case[1])
                hook("Stop", last_assistant_message="An incident conversation is open.")
            elif opt("--round-log", "") and "Call get_supervisor_audit" in prompt:
                audit(tasks[0])
                hook("Stop", last_assistant_message="The audit is ready.")
            elif opt("--round-log", "") and "Call get_supervisor_review" in prompt:
                review(tasks[0])
                hook("Stop", last_assistant_message="The review is ready.")
            elif round_of:
                for task in re.findall(TASK_ID, round_of[3])[:int(round_of[2])]:
                    tool("dispatch_task", {"id": task, "why": "RICE 3×2×1÷1 = 6: the first in line"})
                finish(round_of[1])
                hook("Stop", last_assistant_message="dispatched")
            elif "LIMIT" in prompt:
                hook("StopFailure", error="rate_limit")
            elif "DIES" in prompt:
                sys.exit(3)
            elif "CLEAN" in prompt:
                hook("SessionEnd", reason="other")
                sys.exit(0)
            elif "QUESTIONS" in prompt:
                ask = {"id": tasks[0], "questions": "Why you are needed: only you know which API the clients may use.\nRecommended:\n1. v2: v1 goes away in May. Other options: v1.\nDetails: the task moves the clients to one API; v1 and v2 both work today."}
                if "CHOICES" in prompt:
                    ask.update(kind="operational", recommended="v2", options=[{"id": "v1", "label": "v1"}, {"id": "v2", "label": "<b>v2</b>"}])
                    tool("execute_tool", {"name": "ask_human", "arguments": ask})
                else:
                    tool("ask_human", ask)
                hook("Stop", last_assistant_message="I asked.")
            elif "ASKS" in prompt:
                hook("Notification", notification_type="permission_prompt", message="Approve Bash")
            elif "DEPENDENCY" in prompt:
                tool("wait_for_task", {"id": tasks[0], "on": opt("--wait-on", ""), "why": "The client needs the API."})
                hook("Stop", last_assistant_message="I am waiting for the API.")
            elif "SUBAGENT" in prompt:
                pass
            elif "TOKENS" in prompt:
                path = os.path.join(os.getcwd(), f"transcript-{os.getpid()}.jsonl")
                with open(path, "w", encoding="utf8") as f:
                    f.write(json.dumps({"type": "assistant", "message": {"id": "m1", "model": "claude-opus-5-5", "usage": {"input_tokens": 1200, "output_tokens": 34, "cache_read_input_tokens": 5}}}) + "\n")
                hook("Stop", last_assistant_message="done", transcript_path=path)
                hook("PreToolUse", tool_name="Bash", tool_input={"command": "sleep"})   # then at work: its session stays to be read
            elif "WORKS" in prompt or prompt.startswith("Auto-Dispatcher, which a human turned on"):
                hook("PreToolUse", tool_name="Bash", tool_input={"command": "sleep"})   # a long tool: no Stop
            elif "MONITOR" in prompt:
                box("  ⏵⏵ bypass permissions on · 1 monitor")
                hook("Stop", last_assistant_message="A monitor still runs.")
                while os.path.exists(opt("--monitor-file", "")):
                    time.sleep(1)
                box()
            elif "STUCK" in prompt and main:
                for task in tasks[:-2]:
                    finish(task)
                for task in tasks[-2:]:
                    tool("add_comment", {"id": task, "text": "It cannot be done: the cause is outside."})
                    tool("move_task", {"id": task, "to": "FAILED"})
                hook("Stop", last_assistant_message="stuck")
            elif "FINISHES" in prompt:
                hook("PostToolUse", tool_name="Read", tool_input={}, tool_response={})
                for task in tasks:
                    finish(task)
                hook("Stop", last_assistant_message="done")
            else:
                hook("Stop", last_assistant_message="I did part of it.")
    finally:
        termios.tcsetattr(fd, termios.TCSADRAIN, saved)


if __name__ == "__main__":
    main()
