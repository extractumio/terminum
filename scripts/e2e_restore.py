#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The layout survives a reboot (terminum.md AC-15), end to end against a private terminumd
and tmux server: a checkpoint is written, a "reboot" (another TM_BOOT_ID, the tmux server
gone) brings the sessions, windows, splits and folders back with a report; a restart in the
same boot recreates nothing; layout.json holds no output or environment.

    python3 scripts/e2e_restore.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import subprocess
import sys
import tempfile
import time
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import Terminumd, check, results, summary, tmux_on  # noqa: E402

PORT, SOCKET = 47966, "tm-test-restore"
tmux = partial(tmux_on, SOCKET)


def main():
    t = Terminumd(PORT, SOCKET, env={"TM_BOOT_ID": "boot-A"})
    work = os.path.realpath(tempfile.mkdtemp(prefix="tmr-", dir="/tmp"))
    for d in ("api", "docs", "notes"):
        os.makedirs(os.path.join(work, d))
    try:
        t.start()
        time.sleep(1)
        tmux("rename-window", "-t", "main:", "api")
        tmux("send-keys", "-t", "main:api", f"cd {work}/api", "Enter")
        tmux("split-window", "-t", "main:api", "-c", f"{work}/api")
        tmux("new-window", "-t", "main:", "-n", "docs", "-c", f"{work}/docs")
        tmux("send-keys", "-t", "main:docs", "export SECRET_TOKEN=abc123; echo private-output", "Enter")
        tmux("new-session", "-d", "-s", "notes", "-c", f"{work}/notes")
        layout = os.path.join(t.app, "layout.json")
        saved = None
        for _ in range(30):
            time.sleep(0.5)
            if os.path.exists(layout):
                saved = json.load(open(layout))
                if len(saved["sessions"]) == 2 and sum(len(w["panes"]) for s in saved["sessions"] for w in s["windows"]) == 4:
                    break
        check("AC-15 a checkpoint is written within 5 s of a change", saved and len(saved["sessions"]) == 2, saved and [s["name"] for s in saved["sessions"]])
        text = open(layout).read()
        check("AC-15 layout.json holds no output or environment", "private-output" not in text and "abc123" not in text)
        t.stop()
        tmux("kill-server")
        t.env["TM_BOOT_ID"] = "boot-B"
        t.start()
        ws = t.page()
        report = ws.until(lambda m: m["t"] == "restore", 8)
        check("AC-15 after a reboot the first page gets the report", report is not None and "Restored 2 sessions, 3 windows" in report["report"]["text"], report and report["report"]["text"])
        sessions = tmux("ls", "-F", "#S").split()
        check("AC-15 the sessions are back", sorted(sessions) == ["main", "notes"], sessions)
        wins = tmux("list-windows", "-t", "main", "-F", "#{window_name} #{window_panes}").splitlines()
        check("AC-15 windows and splits are back", wins == ["api 2", "docs 1"], wins)
        paths = set(tmux("list-panes", "-a", "-F", "#{pane_current_path}").split())
        check("AC-15 the panes are in their folders", {f"{work}/api", f"{work}/docs", f"{work}/notes"} <= paths, paths)
        check("AC-15 the report is kept in the state folder", os.path.exists(os.path.join(t.app, "restore-report.json")))
        t.stop()
        t.start()
        time.sleep(1.5)
        check("AC-15 a restart in the same boot recreates nothing", sorted(tmux("ls", "-F", "#S").split()) == ["main", "notes"])
    finally:
        if not all(results):
            print(t.log())
        t.cleanup()
        subprocess.run(["rm", "-rf", work])
    summary()


if __name__ == "__main__":
    main()
