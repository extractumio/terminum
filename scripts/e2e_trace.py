#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""AC-38: repeated traced echoes and a multi-part input, with a private terminumd and tmux."""
import json
from pathlib import Path
import tempfile
import time

from e2e_core import items, screen_with, text_of
from e2e_ws import Terminumd, check, results, summary, wait_layout
from testrun import free_port, own, short_tmp


def keys(frame):
    return (frame.get("tr") or {}).get("ks", [])


def main():
    t = Terminumd(free_port(), own("tm-test-trace"))
    try:
        t.start()
        ws = t.page()
        groups = wait_layout(ws, lambda gs: items(gs), 15)
        ws.send({"t": "sub", "id": items(groups)[0]["id"]})
        ws.until(lambda m: m["t"] == "screen", 15)
        ws.send({"t": "trace", "on": True})
        on = ws.until(lambda m: m["t"] == "trace" and m.get("on") is True, 15)
        check("AC-38 file tracing starts", on is not None)
        for mode in ("file", "stages"):
            missing = []
            for k in range(1, 151):
                time.sleep((k % 7) * 0.005)  # vary arrival across the stream's frame interval
                ws.send({"t": "in", "data": "z" if k % 2 else "\x7f", "k": k})
                got = ws.until(lambda m: m["t"] == "screen" and any(r.get("k") == k for r in keys(m)), 15)
                if got is None:
                    missing.append(k)
                    break
            check(f"AC-38 {mode}: every one of 150 successive keys carries stages in its echo frame", not missing, missing)
            if mode == "file":
                ws.send({"t": "trace", "on": False})
                ws.until(lambda m: m["t"] == "trace" and m.get("on") is False, 15)
                path = Path(t.home, "logs", on["file"])
                body = path.read_text()
                recorded = [r["k"] for r in map(json.loads, body.splitlines()) if r.get("ev") == "key"]
                check("AC-38 every echoed key is written once without its text", recorded == list(range(1, 151)) and '"z"' not in body, recorded)
                ws.send({"t": "trace", "stages": True})
                ws.until(lambda m: m["t"] == "trace" and m.get("stages") is True, 15)

        ws.send({"t": "in", "data": "\x15"})
        # A slow raw reader makes input span many tmux commands and intermediate frames.
        with tempfile.TemporaryDirectory(prefix="tmw-", dir=short_tmp()) as work:
            reader = Path(work, "reader.py")
            reader.write_text(
                "import os, sys, termios, time, tty\n"
                "fd = sys.stdin.fileno(); old = termios.tcgetattr(fd); tty.setraw(fd)\n"
                "try:\n"
                "    os.write(1, b'\\r\\nTRACE_READY\\r\\n'); n = 0\n"
                "    while n < 8192:\n"
                "        n += len(os.read(fd, min(128, 8192 - n)))\n"
                "        os.write(1, ('\\r\\nTRACE_COUNT:%d\\r\\n' % n).encode()); time.sleep(0.05)\n"
                "finally: termios.tcsetattr(fd, termios.TCSADRAIN, old)\n"
                "print('TRACE_DONE')\n")
            ws.send({"t": "in", "data": f"python3 {reader}\r"})
            ready = screen_with(ws, "TRACE_READY", 15)
            check("AC-38 the private slow reader is ready", ready is not None)
            ws.send({"t": "in", "data": "x" * 8192, "k": 501})
            frames = []
            done = ws.until(lambda m: m["t"] == "screen" and (frames.append(m) or "TRACE_DONE" in text_of(m["ch"])), 30)
            traced = [r for frame in frames for r in keys(frame) if r.get("k") == 501]
            progress = [frame for frame in frames if "TRACE_COUNT:" in text_of(frame["ch"]) and "TRACE_DONE" not in text_of(frame["ch"])]
            check("AC-38 a multi-part traced input draws intermediate screens and records its key once", done and progress and len(traced) == 1, len(traced))
        ws.close()
    finally:
        if not all(results):
            print(t.log())
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
