#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""A coding agent as Dispatcher sees it, for web/test/e2e_livedispatcher.mjs: the measured input box of
Claude Code or Codex (terminum-dispatcher.md AS-125), hooks by `terminum hook`, finish_task by `terminum
mcp`; no model. What it does with a prompt is in the prompt's words:
FINISHES, WORKS, SUBAGENT, TOKENS, ASKS, QUESTIONS (ask_human, then its turn ends), LIMIT, DIES, CLEAN, SILENT; any other stops at once. `--stubborn`: SIGTERM is ignored. `--start trust`: a question before its box; `--start
none`: never a box; `--start fast`: exits at once with 7."""
import json
import os
import re
import signal
import subprocess
import sys
import termios
import tty

RULE = "\x1b[37m" + "─" * 60 + "\x1b[0m"
# Longer than this, what is in the box shows as a mark, not its text.
FOLD = 800


def main():
    args = sys.argv[1:]
    cli = args[0]
    opt = lambda name, default: args[args.index(name) + 1] if name in args else default
    kind, start, session = opt("--kind", "claude"), opt("--start", ""), opt("--resume", f"fake-{os.getpid()}")
    typed, pasting = "", False

    def hook(event, **more):
        subprocess.run([cli, "hook", kind], input=json.dumps({"session_id": session, "cwd": os.getcwd(), "hook_event_name": event, **more}), text=True, capture_output=True, timeout=8)

    def draw(body):
        sys.stdout.write("\x1b[2J\x1b[H" + "\r\n".join(body) + "\r\n")
        sys.stdout.flush()

    def box():
        # a long paste folds to a mark, as both agents' boxes do (the gate reads "[Pasted")
        held = typed if len(typed) <= FOLD else (f"[Pasted Content {len(typed)} chars]" if kind == "codex" else f"[Pasted text #1 +{len(typed)} chars]")
        if kind == "codex":
            shown = f"\x1b[1m›\x1b[0m \x1b[2mAsk Codex to do anything\x1b[0m" if not typed else f"\x1b[1m›\x1b[0m {held}"
            draw(["  OpenAI Codex", "", shown, "", "  ? for shortcuts"])
        else:
            draw(["  Claude Code", "", RULE, f"\x1b[39m❯ {held.replace(chr(10), ' ')}\x1b[7m \x1b[0m", RULE, "  ? for shortcuts"])

    def tool(name, arguments):
        rpc = lambda i, m, p=None: json.dumps({"jsonrpc": "2.0", **({} if i is None else {"id": i}), "method": m, **({"params": p} if p else {})})
        lines = [rpc(1, "initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "fake", "version": "1"}}), rpc(None, "notifications/initialized"),
                 rpc(2, "tools/call", {"name": name, "arguments": arguments})]
        subprocess.run([cli, "mcp"], input="\n".join(lines) + "\n", text=True, capture_output=True, timeout=8)

    # a result is kept only in its sections (AC-123): the close-out's and a schedule's, both there
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
        if kind == "claude":
            hook("SessionStart", source="resume" if "--resume" in args else "startup")
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
                elif pasting:
                    typed += part.replace("\r", " ").replace("\n", " ")
                else:
                    ends = ends or "\r" in part
                    if "\x15" in part:                # Ctrl+U clears the box, as both agents' do
                        typed, part = "", part.rsplit("\x15", 1)[1]
                    typed += part.split("\r")[0]
            if not ends:
                box()
                continue
            prompt, typed = typed, ""
            box()
            if "SILENT" in prompt:
                continue
            if kind == "codex":
                hook("SessionStart", source="startup")
            hook("UserPromptSubmit", prompt=prompt)
            several = re.search(r"tasks: ((?:[A-Z][A-Z0-9]*-\d+(?:, )?)+)", prompt)   # a press of several names them all (AC-138)
            tasks = re.findall(r"[A-Z][A-Z0-9]*-\d+", several[1]) if several else re.findall(r"[A-Z][A-Z0-9]*-\d+", prompt)[:1] or [""]
            if "LIMIT" in prompt:
                hook("StopFailure", error="rate_limit")
            elif "DIES" in prompt:
                sys.exit(3)
            elif "CLEAN" in prompt:
                hook("SessionEnd", reason="other")
                sys.exit(0)
            elif "QUESTIONS" in prompt:
                tool("ask_human", {"id": tasks[0], "questions": "1. v1 or v2? Options: v1, v2. Recommended: v2."})
                hook("Stop", last_assistant_message="I asked.")
            elif "ASKS" in prompt:
                hook("Notification", notification_type="permission_prompt", message="Approve Bash")
            elif "SUBAGENT" in prompt:
                hook("SubagentStart", agent_id="sub-1", agent_type="reviewer")
            elif "TOKENS" in prompt:
                path = os.path.join(os.getcwd(), f"transcript-{os.getpid()}.jsonl")
                with open(path, "w", encoding="utf8") as f:
                    f.write(json.dumps({"type": "assistant", "message": {"id": "m1", "model": "claude-opus-5-5", "usage": {"input_tokens": 1200, "output_tokens": 34, "cache_read_input_tokens": 5}}}) + "\n")
                hook("Stop", last_assistant_message="done", transcript_path=path)
                hook("PreToolUse", tool_name="Bash", tool_input={"command": "sleep"})   # then at work: its session stays to be read
            elif "WORKS" in prompt:
                hook("PreToolUse", tool_name="Bash", tool_input={"command": "sleep"})   # a long tool: no Stop
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
