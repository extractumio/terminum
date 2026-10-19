#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Sizing policies and Screen mode end to end, at the page's protocol (terminum.md AC-13
`largest` and `manual`, AC-05 mc on a phone's 48×30) against a private terminumd and a
private tmux server (never the user's). mc runs with a temporary HOME and no subshell.

    python3 scripts/e2e_sizing.py      (needs terminumd built and mc on PATH)
"""
import json
import os
import shutil
import sys
import tempfile
import time
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_core import items, text_of  # noqa: E402
from e2e_ws import Terminumd, check, results, summary, tmux_on, wait_layout  # noqa: E402

PORT, SOCKET = 47963, "tm-test-sizing"
tmux = partial(tmux_on, SOCKET)


def window_of(pane):
    return tmux("display", "-p", "-t", pane, "#{window_width}x#{window_height}").strip()


def settle(pane, want, seconds=4):
    """The window's size once it is `want`, or what it is after `seconds`."""
    end = time.time() + seconds
    while time.time() < end and window_of(pane) != want:
        time.sleep(0.1)
    return window_of(pane)


def policy(t, name):
    with open(os.path.join(t.home, "settings.json"), "w") as f:
        json.dump({"size": name}, f)
    time.sleep(1.6)   # read again within a second (AC-12)


def open_pane(t, pid, cols, rows, typing=False):
    ws = t.page()
    ws.send({"t": "sub", "id": pid})
    ws.send({"t": "size", "cols": cols, "rows": rows, "typing": typing})
    ws.until(lambda m: m["t"] == "screen", 5)
    return ws


def largest(t, pid, pane):
    policy(t, "largest")
    a = open_pane(t, pid, 100, 30)
    check("AC-13 largest: the first page sizes the pane", settle(pane, "100x30") == "100x30", window_of(pane))
    b = open_pane(t, pid, 60, 20)
    b.send({"t": "in", "data": " "})
    time.sleep(1.5)
    check("AC-13 largest: a smaller page that types does not size it", window_of(pane) == "100x30", window_of(pane))
    b.send({"t": "size", "cols": 150, "rows": 40, "typing": False})
    check("AC-13 largest: the page that grows largest sizes it", settle(pane, "150x40") == "150x40", window_of(pane))
    told = a.until(lambda m: m["t"] == "sized" and not m["mine"], 5)
    check("AC-13 largest: the other page is told it reads another screen's size", told and told["cols"] == 150, told)
    b.close()
    time.sleep(0.5)
    a.send({"t": "size", "cols": 100, "rows": 30, "typing": False})
    check("AC-13 largest: once the largest page leaves, the largest left sizes it", settle(pane, "100x30") == "100x30", window_of(pane))
    a.close()


def hidden(t, pid, pane):
    policy(t, "latest")
    mac = open_pane(t, pid, 160, 45)
    check("AC-13 latest: the first page sizes the pane", settle(pane, "160x45") == "160x45", window_of(pane))
    phone = open_pane(t, pid, 103, 21)
    phone.send({"t": "in", "data": " "})
    check("AC-13 latest: the page that types sizes it", settle(pane, "103x21") == "103x21", window_of(pane))
    phone.send({"t": "pause"})
    check("AC-13 latest: a page hidden (a locked phone) gives its window to the page still looking, without a key",
          settle(pane, "160x45") == "160x45", window_of(pane))
    phone.send({"t": "size", "cols": 100, "rows": 5, "typing": False})
    time.sleep(1.5)
    mac.send({"t": "size", "cols": 160, "rows": 30, "typing": False})
    check("AC-13 latest: a hidden page's new size takes nothing; the looking page's split view sizes it",
          settle(pane, "160x30") == "160x30", window_of(pane))
    phone.close()
    mac.close()
    time.sleep(0.5)


def manual(t, pid, pane):
    policy(t, "manual")
    time.sleep(1.2)
    before = window_of(pane)
    c = open_pane(t, pid, 70, 20)
    c.send({"t": "in", "data": " "})
    time.sleep(1.5)
    check("AC-13 manual: opening and typing resize nothing", window_of(pane) == before, (before, window_of(pane)))
    c.send({"t": "fit", "cols": 70, "rows": 20})
    on = c.until(lambda m: m["t"] == "fit" and m["on"], 5)
    check("AC-13 manual: Fit to this screen sizes it now", on and settle(pane, "70x20") == "70x20", window_of(pane))
    c.send({"t": "unfit"})
    off = c.until(lambda m: m["t"] == "fit" and not m["on"], 5)
    check("AC-13 manual: Restore size gives the size it had back", off and settle(pane, before) == before, (before, window_of(pane)))
    c.close()


def drag(t, pid, pane):
    """A dragged window: sizes within a second reach tmux once, the last one."""
    policy(t, "latest")
    d = open_pane(t, pid, 100, 30, typing=True)
    settle(pane, "100x30")
    d.until(lambda m: False, 1.2)   # past the second; the opening's messages read
    d.send({"t": "size", "cols": 96, "rows": 30, "typing": False})
    for cols in (90, 84, 88, 80):
        time.sleep(0.15)
        d.send({"t": "size", "cols": cols, "rows": 30, "typing": False})
    sized, end = [], time.time() + 3
    while time.time() < end:
        m = d.until(lambda m: m["t"] == "sized" and m["mine"], max(0.1, end - time.time()))
        if m:
            sized.append(m["cols"])
    check("AC-13 a dragged window: the passing sizes held back never reach tmux, the last does",
          sized and sized[-1] == 80 and len(sized) <= 2 and not {90, 84, 88} & set(sized) and window_of(pane) == "80x30", (sized, window_of(pane)))
    d.close()


def screen_text(ws, pred, timeout=8):
    m = ws.until(lambda m: m["t"] == "screen" and pred(m), timeout)
    return m


def mc(t, pid, pane, work):
    policy(t, "latest")
    if not shutil.which("mc"):
        check("AC-05 mc is on PATH (brew install midnight-commander)", False)
        return
    p = open_pane(t, pid, 48, 30, typing=True)
    p.send({"t": "in", "data": f"env HOME={work} TERM=tmux-256color mc -u\r"})
    alt = screen_text(p, lambda m: m.get("alt"))
    check("AC-05 mc: the frame says alternate screen", alt is not None)
    p.send({"t": "size", "cols": 48, "rows": 30, "screen": True, "typing": False})
    check("AC-05 mc: Screen mode sizes the pane to the phone's 48x30", settle(pane, "48x30") == "48x30", window_of(pane))
    time.sleep(1.0)
    rows = tmux("capture-pane", "-p", "-t", pane).split("\n")
    check("AC-05 mc: the menu bar is whole at 48 columns", "Left" in rows[0] and "Right" in rows[0], rows[0])
    # mc fits as many function keys as 48 columns hold (six), on the last row
    check("AC-05 mc: the function key bar is the last of 30 rows", len(rows) >= 30 and rows[29].lstrip().startswith("1Help"), rows[29:30])
    p.send({"t": "in", "data": "\x1b[20~"})   # F9: the menu bar, ↓ opens its first menu
    time.sleep(0.4)
    p.send({"t": "in", "data": "\x1b[B"})
    menu = screen_text(p, lambda m: "File listing" in text_of(m["ch"]))
    check("AC-05 mc: F9 and Down open the Left menu", menu is not None)
    p.send({"t": "in", "data": "\x1b\x1b"})
    time.sleep(0.6)
    p.send({"t": "in", "data": "\x1b[21~"})   # F10: quit
    gone = screen_text(p, lambda m: not m.get("alt"))
    check("AC-05 mc: F10 quits and the frame leaves the alternate screen", gone is not None)
    p.close()


def zoomed(t, pid, pane, work):
    target = tmux("split-window", "-t", pane, "-P", "-F", "#{pane_id}").strip()
    time.sleep(1.5)
    ws = t.page()
    groups = wait_layout(ws, lambda gs: any(i["id"].endswith(":" + target) for i in items(gs)))
    sid = next(i["id"] for i in items(groups) if i["id"].endswith(":" + target))
    ws.send({"t": "sub", "id": sid})
    ws.send({"t": "size", "cols": 48, "rows": 30, "typing": True})
    ws.until(lambda m: m["t"] == "screen", 5)
    ws.send({"t": "in", "data": f"env HOME={work} TERM=tmux-256color mc -u\r"})
    screen_text(ws, lambda m: m.get("alt"))
    ws.send({"t": "size", "cols": 48, "rows": 30, "screen": True, "typing": False})
    time.sleep(2.5)   # past a second claim held back by the once-a-second limit
    flag = tmux("display", "-p", "-t", target, "#{window_zoomed_flag} #{pane_width}x#{pane_height}").strip()
    check("AC-05 split window: Screen mode zooms the pane to 48x30", flag == "1 48x30", flag)
    ws.send({"t": "in", "data": "\x1b[21~"})
    screen_text(ws, lambda m: not m.get("alt"))
    ws.send({"t": "size", "cols": 48, "rows": 30, "screen": False, "typing": False})
    time.sleep(1.0)
    flag = tmux("display", "-p", "-t", target, "#{window_zoomed_flag}").strip()
    check("AC-05 split window: the zoom ends with Screen mode", flag == "0", flag)
    # Screen mode on and off again within the once-a-second limit: the window ends unzoomed
    ws.send({"t": "size", "cols": 48, "rows": 30, "screen": True, "typing": False})
    time.sleep(0.3)
    ws.send({"t": "size", "cols": 48, "rows": 30, "screen": False, "typing": False})
    time.sleep(2.0)
    flag = tmux("display", "-p", "-t", target, "#{window_zoomed_flag}").strip()
    check("AC-05 split window: Screen mode on and off within a second leaves it unzoomed", flag == "0", flag)
    ws.close()


def main():
    t = Terminumd(PORT, SOCKET)
    work = tempfile.mkdtemp(prefix="tmz-", dir="/tmp")
    try:
        t.start()
        ws = t.page()
        groups = wait_layout(ws, lambda gs: items(gs))
        pid = items(groups)[0]["id"]
        pane = pid.rsplit(":", 1)[-1]
        ws.close()
        largest(t, pid, pane)
        hidden(t, pid, pane)
        manual(t, pid, pane)
        drag(t, pid, pane)
        mc(t, pid, pane, work)
        zoomed(t, pid, pane, work)
    finally:
        if not all(results):
            print(t.log())
        t.cleanup()
        shutil.rmtree(work, ignore_errors=True)
    summary()


if __name__ == "__main__":
    main()
