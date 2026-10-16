#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Remote hosts end to end (terminum-remote.md AC-81–AC-85, AC-87) against a private
terminumd, with this Mac playing the host through scripts/fake_ssh.py (a private tmux
server, a fake home). Checks: add a host (and refusals), its session in the list within
5 s, typing into a host's pane, a dropped link reconnecting, a host whose ssh fails, enabling
its files (the helper copied, listings through the agent), a paste saved on the host, the
helper removed. Exits non-zero when a check fails.

    python3 scripts/e2e_remote.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import shutil
import signal
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import Terminumd, check, results, summary, unix_request, wait_layout  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
PORT, LOCAL_SOCKET, REMOTE_SOCKET = 47961, "tm-test-rlocal", "tm-test-remote"


def host_group(layout, name):
    return next((g for g in layout if g.get("host") == name), None)


def api(t, secret, path, host=None, method="GET", body=None, headers=None):
    h = {"X-TM-Session": secret, **(headers or {})}
    if host:
        h["X-TM-Host"] = host
    req = urllib.request.Request(f"http://127.0.0.1:{t.port}{path}", data=body, method=method, headers=h)
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, json.loads(e.read() or b"null")


def main():
    t = Terminumd(PORT, LOCAL_SOCKET)
    remote_home = os.path.join(t.root, "remote-home")
    os.makedirs(remote_home)
    open(os.path.join(remote_home, "notes.md"), "w").write("# on the host\n")
    pids = os.path.join(t.root, "ssh-pids")
    t.env.update(TM_SSH=os.path.join(HERE, "fake_ssh.py"), FAKE_SSH_HOME=remote_home, FAKE_SSH_TMUX_SOCKET=REMOTE_SOCKET,
                 FAKE_SSH_PIDS=pids, FAKE_SSH_FAIL="Permission denied (publickey).", FAKE_SSH_FAIL_DEST="refused")
    os.system(f"tmux -L {REMOTE_SOCKET} kill-server 2>/dev/null")
    try:
        t.start()
        secret = t.signin()
        ws = t.page(secret)
        st, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "bad", "recipe": "alex@h -o ProxyCommand=x"})
        check("AC-87 an option not allowed is refused", st == 409 and "option not allowed: -o" in body["message"], body)
        t0 = time.time()
        st, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "", "recipe": "ssh alex@devbox.example -p 2222"})
        check("AC-81 a host is added by its ssh string; no name: its destination's host", st == 200 and body.get("name") == "devbox", body)
        groups = wait_layout(ws, lambda gs: (g := host_group(gs, "devbox")) is not None and g["items"], 10)
        check("AC-81 its terminum session is in the list within 5 s", groups and time.time() - t0 < 5, f"{time.time() - t0:.1f} s")
        g = host_group(groups or [], "devbox") or {"items": [{}]}
        item = g["items"][0]
        check("AC-81 the group and its rows name the host", g.get("label") == "devbox" and item.get("host") == "devbox"
              and item.get("id", "").startswith("tmux:devbox:default:%"), item.get("id"))
        os.system(f"tmux -L {REMOTE_SOCKET} new-session -d -s other -n theirs")   # the host's own session
        last, end = groups, time.time() + 2.5   # the newest layout within that time (one alike is not sent again)
        while time.time() < end:
            last = wait_layout(ws, lambda gs: True, max(0.1, end - time.time())) or last
        groups = last or []
        devbox = [g for g in groups if g.get("host") == "devbox"]
        check("AC-81 only the session terminum of the host is listed, with how to attach to it",
              len(devbox) == 1 and devbox[0].get("session") == "terminum" and not any(i.get("title") == "theirs" for i in devbox[0]["items"])
              and devbox[0].get("attach") == "ssh -t -p 2222 -- alex@devbox.example 'tmux attach -t =terminum'", devbox)
        recs = unix_request(t.sock, "GET", "/hosts")[1]["hosts"]
        check("AC-87 the record keeps the checked recipe", recs and recs[0]["ssh"] == ["-p", "2222", "alex@devbox.example"], recs)

        ws.send({"t": "sub", "id": item["id"]})
        ws.send({"t": "size", "cols": 90, "rows": 25, "typing": False})
        check("AC-81 a host's pane shows its screen", ws.until(lambda m: m["t"] == "screen", 10) is not None)
        ws.send({"t": "in", "data": "echo remote-hello-$((6*7))\r"})
        echo = ws.until(lambda m: m["t"] == "screen" and "remote-hello-42" in json.dumps(m["ch"]), 10)
        check("AC-81 typing reaches the host's pane", echo is not None)

        # the link drops: the control client's ssh ends
        for line in open(pids):
            pid, _, rest = line.partition(" ")
            if "new-session" in rest:
                try:
                    os.kill(int(pid), signal.SIGTERM)
                except ProcessLookupError:
                    pass
        lost = wait_layout(ws, lambda gs: (g := host_group(gs, "devbox")) is not None and g.get("state") in ("reconnecting", "failed"), 10)
        check("AC-81 a dropped link says reconnecting", lost is not None, (host_group(lost or [], "devbox") or {}).get("why"))
        back = wait_layout(ws, lambda gs: (g := host_group(gs, "devbox")) is not None and g.get("state") == "attached" and g["items"], 15)
        check("AC-81 the host comes back by itself", back is not None)
        ws.send({"t": "in", "data": "echo again-$((2+3))\r"})
        check("AC-81 typing works again after the reconnect",
              ws.until(lambda m: m["t"] == "screen" and "again-5" in json.dumps(m["ch"]), 10) is not None)

        st, _ = unix_request(t.sock, "POST", "/hosts/add", {"name": "refused", "recipe": "alex@refused.example"})
        failed = wait_layout(ws, lambda gs: (g := host_group(gs, "refused")) is not None and g.get("state") == "failed", 10)
        why = (host_group(failed or [], "refused") or {}).get("why", "")
        check("AC-81 an ssh that fails says why", "Permission denied (publickey)" in why, why)
        unix_request(t.sock, "POST", "/hosts/remove", {"name": "refused"})

        st, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "", "recipe": "alex@devbox.example -p 2200"})
        check("AC-81 a name made that another host has is refused", st == 409 and "devbox is already a host" in body["message"], body)

        # AC-83: files ticked in Add host are set up once the host is attached; a failure asks again
        def offer_until(pred):
            remote = {}
            for _ in range(80):
                remote = (api(t, secret, "/fb/api/state?host=devbox")[1] or {}).get("remote") or {}
                if pred(remote):
                    break
                time.sleep(0.25)
            return remote
        unix_request(t.sock, "POST", "/hosts/remove", {"name": "devbox"})
        blocker = os.path.join(remote_home, ".terminum")
        shutil.rmtree(blocker, ignore_errors=True)
        open(blocker, "w").close()   # a file where the helper's folder goes: the copy fails
        st, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "", "recipe": "alex@devbox.example -p 2222", "files": True})
        remote = offer_until(lambda r: r.get("why"))
        rec = next((h for h in unix_request(t.sock, "GET", "/hosts")[1]["hosts"] if h["name"] == "devbox"), {})
        check("AC-83 a ticked setup that fails clears the tick; Files asks again with the reason",
              st == 200 and remote.get("state") == "ask" and "Could not enable devbox" in remote.get("why", "") and rec.get("files") is None, (remote, rec))
        os.remove(blocker)
        st, body = unix_request(t.sock, "POST", "/hosts/add", {"name": "", "recipe": "alex@devbox.example -p 2222", "files": True})
        remote = offer_until(lambda r: r.get("state") == "up")
        rec = next((h for h in unix_request(t.sock, "GET", "/hosts")[1]["hosts"] if h["name"] == "devbox"), {})
        check("AC-83 files ticked in Add host: the helper is copied and files are enabled",
              st == 200 and remote.get("state") == "up" and rec.get("files") == "enabled"
              and os.path.exists(os.path.join(remote_home, ".terminum/bin/terminumd-agent")), (remote, rec))
        listing = None
        for _ in range(60):
            st, listing = api(t, secret, f"/fb/api/ls?path={remote_home}", host="devbox")
            if st == 200:
                break
            time.sleep(0.25)
        names = [e["n"] for e in (listing or {}).get("entries", [])] if st == 200 else listing
        check("AC-82 a host's folder is listed through its agent", st == 200 and "notes.md" in names, names)
        st, body = api(t, secret, f"/fb/api/file?path={remote_home}/notes.md", host="devbox")
        check("AC-82 a host's file is read through its agent", st == 200 and body.get("text") == "# on the host\n", body)
        st, body = api(t, secret, "/fb/api/os/reveal", host="devbox", method="POST", body=json.dumps({"path": remote_home}).encode(),
                       headers={"Content-Type": "application/json"})
        check("AC-82 Finder actions are refused for remote files", st == 400 and "remote" in json.dumps(body).lower(), body)
        png = b"\x89PNG\r\n\x1a\n" + b"\0" * 32
        st, body = api(t, secret, "/paste?sid=" + urllib.request.quote(item["id"]), method="POST", body=png,
                       headers={"Content-Type": "image/png", "Origin": f"http://127.0.0.1:{PORT}"})
        path = (body or {}).get("path", "")
        check("AC-85 a pasted image is saved on the host", st == 200 and path.startswith(remote_home + "/.cache/terminum/paste/") and os.path.exists(path), body)

        # a host set up before the server was named terminumd has only terminusd-agent: it is
        # started, answers with its old build, and is replaced by this build's helper (AC-84)
        t.stop()
        bin_dir = os.path.join(remote_home, ".terminum/bin")
        for f in os.listdir(bin_dir):
            os.remove(os.path.join(bin_dir, f))
        with open(os.path.join(bin_dir, "terminusd-agent-0123456789ab"), "w") as f:
            f.write("#!/bin/sh\necho terminusd-agent ready 0123456789ab h u darwin-aarch64\nexec cat >/dev/null\n")
        os.chmod(os.path.join(bin_dir, "terminusd-agent-0123456789ab"), 0o755)
        os.symlink("terminusd-agent-0123456789ab", os.path.join(bin_dir, "terminusd-agent"))
        t.start()
        ws = t.page(secret)
        remote = offer_until(lambda r: r.get("state") == "up")
        left = sorted(os.listdir(bin_dir))
        check("AC-84 a helper of the old name is replaced by terminumd-agent, its files removed",
              remote.get("state") == "up" and "terminumd-agent" in left and not any(n.startswith("terminusd-agent") for n in left), (remote, left))
        st, body = unix_request(t.sock, "POST", "/hosts/files-remove", {"name": "devbox"})
        check("AC-83 the helper is removed from the host", st == 200 and not os.path.exists(os.path.join(remote_home, ".terminum/bin/terminumd-agent")), body)
        st, body = unix_request(t.sock, "POST", "/hosts/remove", {"name": "devbox"})
        gone = wait_layout(ws, lambda gs: host_group(gs, "devbox") is None, 10)
        check("AC-87 a removed host leaves the list", st == 200 and gone is not None)
    finally:
        if not all(results):
            print(t.log())
        t.cleanup()
        os.system(f"tmux -L {REMOTE_SOCKET} kill-server 2>/dev/null")
    summary()


if __name__ == "__main__":
    main()
