#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Full-load bench (terminum.md AC-195), by hand, not in `make check`: a private terminumd and tmux,
first quiet, then beside busy loops (4 a core: CPU, memory, system calls) at a terminal program's
priority and tmux windows printing without a pause. In each phase
it lists the board, edits a card and types keys into a shell, all at once, and prints each one's
median and p95, with the AC-195 lines (`store.slow`, `tmux.slow`, `runtime.lag`) its terminumd wrote.
The loops end by themselves after `--seconds` + 30 and are ended by pid on pass, fail or signal.

    python3 scripts/bench_busy.py [--seconds 30] [--loops <4 x cores>] [--printers 8] [--tasks 200] [--nice 0] [--installed]
"""
import argparse
import collections
import json
import math
import os
import statistics
import subprocess
import sys
import threading
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import Terminumd, tmux_on, unix_request  # noqa: E402
from e2e_remote import api  # noqa: E402
from testrun import free_port, own  # noqa: E402

PORT, SOCKET = free_port(), own("tm-test-bench-busy")
LINES = ("store.slow", "tmux.slow", "runtime.lag")


def shown(m):
    """The keys a frame says it shows, with their stages (AC-38)."""
    tr = m.get("tr")
    return [k for k in (tr.get("ks") or []) if isinstance(k, dict)] if isinstance(tr, dict) else []


def p95(xs):
    return sorted(xs)[max(0, math.ceil(len(xs) * 0.95) - 1)] if xs else float("nan")


def timed(times, f):
    t0 = time.perf_counter()
    if f():
        times.append((time.perf_counter() - t0) * 1000)


def board(call, until, times):
    while time.time() < until:
        timed(times, lambda: call("/api/tasks")[0] == 200)


def edits(call, ids, until, times):
    i = 0
    while time.time() < until:
        task = ids[i % len(ids)]
        timed(times, lambda: call(f"/api/tasks/{task}", "PATCH", {"title": f"bench card {i}"})[0] == 200)
        i += 1
        time.sleep(0.05)


def echo(ws, until, times, stages):
    """Keys into the shell, each timed to the frame that says it shows it (AC-38's key stages)."""
    i = 0
    while time.time() < until:
        i += 1
        if i % 100 == 0:
            ws.send({"t": "in", "data": "\x15"})
        t0 = time.perf_counter()
        ws.send({"t": "in", "data": "abcdefghijklmnopqrstuvwxyz"[i % 26], "k": i})
        mine = lambda m: next((k for k in shown(m) if k.get("k") == i), None) if m["t"] == "screen" else None  # noqa: E731
        m = ws.until(mine, 10)
        times.append((time.perf_counter() - t0) * 1000 if m else 10000.0)   # never shown: the wait's end
        if m:
            stages.append(mine(m))


def phase(name, call, ws, ids, seconds, installed):
    got = {"board list": [], "card edit": [], "key echo": []}
    stages = []
    until = time.time() + seconds
    workers = [threading.Thread(target=board, args=(call, until, got["board list"])),
               threading.Thread(target=edits, args=(call, ids, until, got["card edit"]))]
    if installed:
        got["installed /health"] = []
        workers.append(threading.Thread(target=live, args=(until, got["installed /health"])))
    for w in workers:
        w.start()
    echo(ws, until, got["key echo"], stages)
    for w in workers:
        w.join()
    for what, xs in got.items():
        med = statistics.median(xs) if xs else float("nan")
        print(f"{name:6} {what:11} n={len(xs):5}  median {med:7.1f} ms  p95 {p95(xs):7.1f} ms  max {max(xs, default=float('nan')):7.1f} ms")
    for stage in ("to_tmux_ms", "until_change_ms", "polls", "capture_ms", "encode_ms", "total_ms"):
        xs = [k[stage] for k in stages if isinstance(k.get(stage), (int, float))]
        if xs:
            print(f"{name:6}   key {stage:15} median {statistics.median(xs):7.1f}  p95 {p95(xs):7.1f}  max {max(xs):7.1f}")
    print(f"{name:6}   key woken by {dict(collections.Counter(k.get('woken') for k in stages))}")


def loops(n, seconds, nice):
    """n busy loops at `nice`, each ending by itself after `seconds`: CPU, memory and system calls, as a build's."""
    code = (f"import hashlib, os, time\nos.nice({nice})\nend = time.time() + {seconds}\n"
            "while time.time() < end:\n    hashlib.sha256(bytes(os.urandom(1 << 16)) * 64).digest()\n")
    return [subprocess.Popen([sys.executable, "-I", "-c", code]) for _ in range(n)]


def printers(socket, n):
    """n windows of the private tmux server printing without a pause, as a build's log."""
    for i in range(n):
        tmux_on(socket, "new-window", "-d", "-n", f"print{i}", "sh -c 'while :; do ls -la /usr/bin; done'")


def live(until, times):
    """The installed terminumd's /health on its socket, timed (read only): launchd's service beside the test's."""
    path = os.path.expanduser("~/.terminum/state/terminumd.sock")
    while os.path.exists(path) and time.time() < until:
        timed(times, lambda: unix_request(path, "GET", "/health")[0] == 200)
        time.sleep(0.2)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--seconds", type=int, default=30)
    ap.add_argument("--loops", type=int, default=4 * os.cpu_count())
    ap.add_argument("--printers", type=int, default=8, help="private tmux windows printing without a pause")
    ap.add_argument("--installed", action="store_true", help="also time the installed terminumd's /health")
    ap.add_argument("--tasks", type=int, default=200)
    ap.add_argument("--nice", type=int, default=0, help="the loops' nice: 0 as a program in a terminal, 10 as a nice'd build")
    a = ap.parse_args()
    t = Terminumd(PORT, SOCKET, env={"TM_LOG": "warn"})
    busy = []
    try:
        t.start()
        secret = t.signin()
        headers = {"Content-Type": "application/json", "Origin": f"http://127.0.0.1:{PORT}"}
        call = lambda path, method="GET", body=None: api(t, secret, path, method=method,  # noqa: E731
                                                         body=json.dumps(body).encode() if body is not None else None, headers=headers)
        ids = [call("/api/tasks", "POST", {"title": f"bench card {i}", "text": "x" * 400})[1]["id"] for i in range(a.tasks)]
        ws = t.page(secret)
        g = ws.until(lambda m: m["t"] == "layout" and m["groups"] and m["groups"][0]["items"], 5)["groups"]
        ws.send({"t": "sub", "id": g[0]["items"][0]["id"]})
        ws.send({"t": "size", "cols": 200, "rows": 60, "typing": False})
        ws.until(lambda m: m["t"] == "screen", 5)
        ws.send({"t": "trace", "stages": True})
        ws.send({"t": "in", "data": "clear\r"})
        time.sleep(1)
        phase("quiet", call, ws, ids, a.seconds, a.installed)
        busy = loops(a.loops, a.seconds + 30, a.nice)
        printers(SOCKET, a.printers)
        time.sleep(2)
        phase("busy", call, ws, ids, a.seconds, a.installed)
        logged = open(t.log_path).readlines()
        for e in LINES:
            mine = [ln for ln in logged if f'event="{e}"' in ln]
            print(f"{e}: {len(mine)} lines" + "".join("\n  " + ln.strip().split(" ", 1)[-1][:220] for ln in mine[-5:]))
    finally:
        for p in busy:
            if p.poll() is None:
                p.terminate()
        for p in busy:
            p.wait(5)
        t.cleanup()


if __name__ == "__main__":
    main()
