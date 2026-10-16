#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""The core end to end, at the page's protocol (terminum.md AC-01, AC-02, AC-03, AC-07,
AC-13, AC-14) against a private terminumd and a private tmux server (never the user's):
the list follows tmux, screens and history, typing, keys by name, pastes, sessions, sizing,
a killed tmux server. Browser checks of the page are in web/test.

    python3 scripts/e2e_core.py      (needs terminumd built: cd terminumd && cargo build)
"""
import os
import subprocess
import sys
import tempfile
import time
from functools import partial

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import Terminumd, check, results, summary, tmux_on, wait_layout  # noqa: E402

PORT, SOCKET = 47962, "tm-test-core"
tmux = partial(tmux_on, SOCKET)


def items(groups):
    return [it for g in groups for it in g["items"]]


def text_of(ch):
    out = []
    for _, line in ch:
        out.append("".join(r[0] if isinstance(r[0], str) else "".join(r[0]) for r in line["r"]))
    return "\n".join(out)


def screen_with(ws, needle, timeout=5):
    return ws.until(lambda m: m["t"] == "screen" and needle in text_of(m["ch"]), timeout)


def main():
    t = Terminumd(PORT, SOCKET)
    work = tempfile.mkdtemp(prefix="tmw-", dir="/tmp")
    odd = os.path.join(work, "it's #(id) $HOME")
    os.makedirs(odd)
    try:
        t.start()
        ws = t.page()
        groups = wait_layout(ws, lambda gs: items(gs))
        check("AC-01 the first start makes a session main", groups and groups[0]["label"] == "main", [g["label"] for g in groups or []])
        check("AC-01 the user's default tmux server is untouched", "tm-test" not in subprocess.run(["tmux", "ls"], capture_output=True, text=True).stdout)
        first = items(groups)[0]
        ws.send({"t": "sub", "id": first["id"]})
        ws.send({"t": "size", "cols": 80, "rows": 24, "typing": False})
        hist = ws.until(lambda m: m["t"] == "hist", 5)
        check("AC-02 opening a pane sends its history then its screen", hist and hist["mode"] == "reset" and ws.until(lambda m: m["t"] == "screen", 5))
        sized = ws.until(lambda m: m["t"] == "sized", 5)
        check("AC-13 the page that opens a pane sizes it", sized and sized["cols"] == 80 and sized["mine"], sized)
        check("AC-13 the window is 80x24 with window-size manual", "80x24 manual" in tmux("list-windows", "-F", "#{window_width}x#{window_height} #{window-size}"))

        t0 = time.time()
        ws.send({"t": "in", "data": f"cd '{work}' && echo typed-$((40+2))\r", "k": 1})
        echo = screen_with(ws, "typed-42")
        check("AC-03 typed text and Enter reach the pane", echo is not None, f"{(time.time() - t0) * 1000:.0f} ms")
        ws.send({"t": "in", "data": "printf '<%s>' ab\x1b[D\x1b[3~X\r"})
        check("AC-03 special keys go by name (Left, DC)", screen_with(ws, "<aX>") is not None)

        ws.send({"t": "in", "data": "for i in $(seq 1 3000); do echo line-$i; done\r"})
        appended, last = 0, 0
        end = time.time() + 15
        while time.time() < end:
            m = ws.recv(5)
            if m["t"] == "hist" and m["mode"] == "append":
                appended += len(m["lines"])
                last = m["first"] + len(m["lines"])
            if m["t"] == "screen" and "line-3000" in text_of(m["ch"]):
                break
        check("AC-02 lines that scroll off are appended with their numbers", appended >= 2900 and last > 2900, f"{appended} rows, top {last}")
        ws.send({"t": "more", "before": last - 500})
        older = ws.until(lambda m: m["t"] == "hist" and m["mode"] == "prepend", 5)
        check("AC-02 older lines load on request", older and len(older["lines"]) > 0 and older["first"] + len(older["lines"]) == last - 500, older and (older["first"], len(older["lines"])))
        ws.send({"t": "in", "data": "clear && tmux clear-history\r"})
        reset = ws.until(lambda m: m["t"] == "hist" and m["mode"] == "reset", 5)
        check("AC-02 a cleared scrollback starts the page's history anew", reset is not None)

        # lines under the terminal's canonical limit (1024 bytes a line), 20 KB in all
        big = "\n".join(f"line-{i:04d}-" + "y" * 40 for i in range(400))
        ws.send({"t": "in", "data": f"cat > {work}/pasted.txt\r"})
        time.sleep(0.5)
        ws.send({"t": "in", "data": f"\x1b[200~{big}\x1b[201~"})
        ws.send({"t": "in", "data": "\r\x04"})
        time.sleep(1.5)
        got = open(f"{work}/pasted.txt").read() if os.path.exists(f"{work}/pasted.txt") else ""
        check("AC-03 a 20 KB paste goes through a tmux buffer", big in got, f"{len(got)} bytes")
        ws.send({"t": "in", "data": "\x1b[200~bad\x07paste\x1b[201~"})
        err = ws.until(lambda m: m["t"] == "error" and "control characters" in m.get("msg", ""), 5)
        check("AC-03 a paste with control characters is refused", err is not None)

        # AC-07: a new window after the shown one, in its folder (an odd name stays one argument)
        quoted = "'" + odd.replace("'", "'\\''") + "'"
        ws.send({"t": "in", "data": f"cd {quoted}\r"})
        time.sleep(0.8)
        ws.send({"t": "new", "group": groups[0]["wid"]})
        created = ws.until(lambda m: m["t"] in ("created", "error"), 5)
        path = tmux("display", "-p", "-t", (created or {}).get("id", "x").rsplit(":", 1)[-1], "#{pane_current_path}").strip()
        check("AC-07 a new window opens in the shown pane's folder", created and created["t"] == "created" and path == os.path.realpath(odd), path)
        check("AC-07 #(…) in a folder name ran nothing", not os.path.exists(os.path.join(work, "id")))
        time.sleep(1.1)
        ws.send({"t": "split", "id": created["id"], "how": "below"})
        split = ws.until(lambda m: m["t"] in ("created", "error"), 5)
        check("AC-07 a split opens next to the pane", split and split["t"] == "created", split)
        time.sleep(1.1)
        ws.send({"t": "newsession", "name": "bad:name"})
        bad = ws.until(lambda m: m["t"] == "error", 5)
        check("AC-07 a session name tmux refuses is refused in words", bad is not None, bad and bad["msg"])
        time.sleep(1.1)
        ws.send({"t": "newsession", "name": "notes"})
        made = ws.until(lambda m: m["t"] in ("created", "error"), 5)
        check("AC-07 a new session is made", made and made["t"] == "created" and "notes" in tmux("ls", "-F", "#S"), made)
        ws.send({"t": "rename", "id": created["id"], "name": "dep#(id)loy"})
        renamed = wait_layout(ws, lambda gs: any(i["title"] == "dep#(id)loy" for i in items(gs)))
        check("AC-07 a rename shows in the list, # kept literally", renamed is not None, tmux("list-windows", "-a", "-F", "#{window_name}"))
        ws.send({"t": "move", "id": created["id"], "session": "local/notes"})
        moved = wait_layout(ws, lambda gs: any(g["label"] == "notes" and any(i["id"] == created["id"] for i in g["items"]) for g in gs))
        check("AC-07 a window moves to another session", moved is not None)
        ws.send({"t": "kill", "id": split["id"]})
        killed = wait_layout(ws, lambda gs: not any(i["id"] == split["id"] for i in items(gs)))
        check("AC-07 a closed pane leaves the list", killed is not None)
        tmux("new-window", "-d", "-t", "main:", "-n", "byhand")
        check("AC-01 a window made by hand shows within 2 s", wait_layout(ws, lambda gs: any(i["title"] == "byhand" for i in items(gs)), 3) is not None)

        # AC-38: ?trace=1 times a key from its arrival to the frame that shows it, into logs/
        ws.send({"t": "trace", "on": True})
        on = ws.until(lambda m: m["t"] == "trace" and "on" in m, 5)
        ws.send({"t": "in", "data": "z", "k": 77})
        traced = ws.until(lambda m: m["t"] == "screen" and any(k.get("k") == 77 for k in (m.get("tr") or {}).get("ks", [])), 5)
        ws.send({"t": "in", "data": "\x15"})
        ws.send({"t": "trace", "on": False})
        ws.until(lambda m: m["t"] == "trace", 5)
        logs = os.path.join(t.home, "logs")
        files = [f for f in os.listdir(logs) if f.startswith("trace-")] if os.path.isdir(logs) else []
        body = open(os.path.join(logs, files[0])).read() if files else ""
        check("AC-38 a traced key comes back with its stages in the frame", on and on.get("on") and traced is not None,
              traced and traced["tr"]["ks"][0])
        check("AC-38 the trace is written without typed text", files and '"k":77' in body.replace(" ", "") and '"z"' not in body, files)

        # AC-06: a program that asked for the mouse gets SGR reports for clicks and the wheel
        reader = os.path.join(work, "mouse.py")
        with open(reader, "w") as f:
            f.write("import os, sys, termios, tty\n"
                    "fd = sys.stdin.fileno(); old = termios.tcgetattr(fd); tty.setraw(fd)\n"
                    "sys.stdout.write('\\x1b[?1049h\\x1b[?1000h\\x1b[?1002h\\x1b[?1006h'); sys.stdout.flush()\n"
                    "got = b''\n"
                    "while got.count(b'M') + got.count(b'm') < 3: got += os.read(fd, 64)\n"
                    "termios.tcsetattr(fd, termios.TCSADRAIN, old)\n"
                    "sys.stdout.write('\\x1b[?1006l\\x1b[?1002l\\x1b[?1000l\\x1b[?1049l')\n"
                    "print('MOUSE', got.decode().replace('\\x1b', 'ESC'))\n")
        ws.send({"t": "in", "data": f"python3 {reader}\r"})
        alt = ws.until(lambda m: m["t"] == "screen" and m.get("alt") and m.get("mouse"), 8)
        check("AC-06 a frame says the program asked for the mouse", alt is not None)
        ws.send({"t": "mouse", "x": 9, "y": 4, "button": 0, "action": "press", "mods": {}})
        ws.send({"t": "mouse", "x": 9, "y": 4, "button": 0, "action": "release", "mods": {}})
        ws.send({"t": "mouse", "x": 2, "y": 3, "button": 0, "action": "wheeldown", "mods": {}})
        got = screen_with(ws, "MOUSE", 8)
        line = text_of(got["ch"]) if got else ""
        check("AC-06 clicks and the wheel arrive as SGR mouse reports (1-based cells)",
              "ESC[<0;10;5M" in line and "ESC[<0;10;5m" in line and "ESC[<65;3;4M" in line, line.strip()[-80:])

        # AC-13: a second page that types takes the size
        other = t.page()
        other.until(lambda m: m["t"] == "layout", 5)
        other.send({"t": "sub", "id": first["id"]})
        other.send({"t": "size", "cols": 48, "rows": 30, "typing": False})
        time.sleep(1.2)
        check("AC-13 a page that only reads never resizes", "80x24" in tmux("display", "-p", "-t", first["id"].rsplit(":", 1)[-1], "#{window_width}x#{window_height}"))
        other.send({"t": "in", "data": " "})
        mine = other.until(lambda m: m["t"] == "sized" and m["mine"], 5)
        check("AC-13 the page that typed last sizes the pane", mine and (mine["cols"], mine["rows"]) == (48, 30), mine)
        theirs = ws.until(lambda m: m["t"] == "sized" and not m["mine"], 5)
        check("AC-13 the other page is told it was sized for another screen", theirs is not None)
        other.close()

        # AC-14: the tmux server goes away
        tmux("kill-server")
        seen = {"closed": False, "main": False}

        def note(m):
            seen["closed"] |= m["t"] == "error" and "closed" in m.get("msg", "")
            seen["main"] |= m["t"] == "layout" and any(g["label"] == "main" and not any(i["id"] == created["id"] for i in g["items"]) for g in m["groups"])
            return all(seen.values())
        ws.until(note, 8)
        check("AC-14 a shown pane says it has closed", seen["closed"])
        check("AC-14 the server is started again with a session main", seen["main"])
        h = t.health()
        check("AC-08 status names the build, port and tmux", h["port"] == PORT and h["tmux"]["ok"] and h["build"], {k: h[k] for k in ("build", "port", "pages")})
    finally:
        if not all(results):
            print(t.log())
        t.cleanup()
        subprocess.run(["rm", "-rf", work])
    summary()


if __name__ == "__main__":
    main()
