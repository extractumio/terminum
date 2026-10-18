#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Coding agents' states in the list (terminum-terminal.md AC-39) for Codex installed with npm,
whose program is `node`, against a private terminumd and a private tmux server (never the
user's): `node` programs draw Codex's screens as measured on Codex 0.162, and a dev server's.

    python3 scripts/e2e_agentstate.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import shutil
import sys
import tempfile
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import Terminumd, check, summary, tmux_on, wait_layout  # noqa: E402

PORT, SOCKET = 47970, "tm-test-agent"
tmux = partial(tmux_on, SOCKET)

# what each stand-in prints at the screen's bottom, as Codex draws, then it waits; "later" clears
# the screen after 3 s and prints that
SCREENS = {
    "idle": {"rows": ["  >_ OpenAI Codex (v0.162.1)", "", "› Ask Codex to do anything", "  ? for shortcuts"]},
    "working": {"rows": ["• Ran pwd", "", "Working (10s • esc to interrupt)", "", "› Ask Codex to do anything"]},
    "ask": {"title": "[ ! ] Action Required | Pick a language | p",
            "rows": ["  Question 1/1 (1 unanswered)", "  Which language?", "  › 1. Python", "    2. Bash", "",
                     "  tab to add notes | enter to submit answer | esc to interrupt"]},
    "dev": {"rows": ["  ready - started server on 0.0.0.0:3000",
                     "? Would you like to run the app on another port instead? › (Y/n)"]},
    "switch": {"rows": ["• Created essay.md", "  Worked for 43s • 11:04", "", "› Ask Codex to do anything"],
               "later": ["  ready - started server on 0.0.0.0:3000", "? Would you like to run the app on another port instead? › (Y/n)"]},
}

PROGRAM = r"""
const s = JSON.parse(process.argv[2]);
const draw = (rows) => process.stdout.write(`\x1b[2J\x1b[${process.stdout.rows - rows.length + 1};1H` + rows.join("\r\n"));
if (s.title) process.stdout.write(`\x1b]2;${s.title}\x07`);
draw(s.rows);
if (s.later) setTimeout(() => { process.stdout.write("\x1b]2;dev\x07"); draw(s.later); }, 3000);
setInterval(() => {}, 1000);
"""


def items(groups):
    return [it for g in groups for it in g["items"]]


def row(groups, title):
    return next((i for i in items(groups) if i["index"] == title), None)


def main():
    t = Terminumd(PORT, SOCKET)
    work = tempfile.mkdtemp(prefix="tma-", dir="/tmp")
    program = os.path.join(work, "standin.js")
    with open(program, "w") as f:
        f.write(PROGRAM)
    try:
        t.start()
        ws = t.page()
        last = [[]]   # the layout is sent only when it changed: the newest one stands

        def newest(pred, timeout=5):
            def keep(gs):
                last[0] = gs
                return pred(gs)
            return wait_layout(ws, keep, timeout)
        newest(items)
        index = {}

        def run(name):
            out = tmux("new-window", "-d", "-P", "-F", "#{window_index}", "-t", "main", "node", program, json.dumps(SCREENS[name]))
            index[name] = out.strip()
        for name in SCREENS:
            if name != "switch":
                run(name)
        at = lambda name: (lambda gs: row(gs, index[name]))  # noqa: E731

        def seen(name, pred, timeout=10):
            def holds(gs):
                return (r := at(name)(gs)) is not None and pred(r)
            if holds(last[0]) or newest(holds, timeout):
                return at(name)(last[0])
            print("  last row:", at(name)(last[0]))
            return None

        newest(lambda gs: len(items(gs)) > len(SCREENS) - 1)
        programs = tmux("list-panes", "-s", "-t", "main", "-F", "#{window_index} #{pane_current_command}")
        check("AC-39 the stand-ins run as node, as npm's Codex does", all(f"{i} node" in programs for i in index.values()), programs.split("\n"))
        idle = seen("idle", lambda r: r["agent"] == "codex")
        check("AC-39 a node pane showing Codex's composer is Codex, done shows no mark", idle and idle["state"] == "done", idle)
        working = seen("working", lambda r: r["state"] == "working")
        check("AC-39 Codex working: \"esc to interrupt\", its title unchanged", working and working["agent"] == "codex", working)
        ask = seen("ask", lambda r: r["state"] == "waiting")
        check("AC-39 Codex's question waits over \"esc to interrupt\"", ask and ask["agent"] == "codex", ask)
        check("AC-39 the list title leaves out Codex's \"Action Required\" mark", ask and ask["title"] == "Pick a language | p", ask)
        dev = seen("dev", lambda r: True)
        check("AC-39 another node program asking a question shows no agent and no state",
              dev and dev["agent"] is None and dev["state"] is None, dev)
        run("switch")    # its screen changes 3 s after it starts
        before = seen("switch", lambda r: r["agent"] == "codex")
        check("AC-39 Codex in a pane before the switch", before and before["state"] == "done", before)
        after = seen("switch", lambda r: r["agent"] is None, 12)
        check("AC-39 a node pane leaving Codex for a dev server stops being Codex (no false reminder)",
              after and after["state"] is None, after)
    finally:
        t.stop()
        tmux("kill-server")
        t.cleanup()
        shutil.rmtree(work, ignore_errors=True)
    summary()


if __name__ == "__main__":
    main()
