#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
"""Who may use terminumd, end to end (terminum.md AC-09, AC-10) against a private terminumd:
the table of refused requests, the one-time code, the hello proof, the private socket and
files, web access with its password, cookie, brakes and switch-off.

    python3 scripts/e2e_access.py      (needs terminumd built: cd terminumd && cargo build)
"""
import json
import os
import socket
import stat
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from e2e_ws import WS, Terminumd, check, proof, results, summary, unix_request  # noqa: E402

PORT, WEB_PORT, SOCKET = 47964, 47965, "tm-test-access"


def raw(port, method, path, headers=None, body=b""):
    """(status, JSON body) of a request sent exactly as given (urllib would set Host itself)."""
    s = socket.create_connection(("127.0.0.1", port), timeout=10)
    head = [f"{method} {path} HTTP/1.1", "Connection: close", f"Content-Length: {len(body)}"]
    head += [f"{k}: {v}" for k, v in (headers or {}).items()]
    s.sendall(("\r\n".join(head) + "\r\n\r\n").encode() + body)
    data = b""
    while chunk := s.recv(65536):
        data += chunk
    s.close()
    head, _, rest = data.partition(b"\r\n\r\n")
    status = int(head.split(b" ")[1])
    try:
        return status, json.loads(rest or b"null"), head.decode()
    except ValueError:
        return status, rest.decode(errors="replace"), head.decode()


def main():
    t = Terminumd(PORT, SOCKET)
    try:
        t.start()
        secret = t.signin()
        host = f"127.0.0.1:{PORT}"
        sess = {"Host": host, "X-TM-Session": secret}
        cases = [
            ("GET /ws (no sign-in)", "GET", "/ws", {"Host": host, "Upgrade": "websocket", "Connection": "Upgrade", "Origin": f"http://{host}"}, 401, "bad_session"),
            ("GET /fb/api/ls, Host: evil.example", "GET", "/fb/api/ls?path=/tmp", {**sess, "Host": f"evil.example:{PORT}"}, 403, "bad_host"),
            ("GET /ws with a sign-in, Origin: http://evil.example", "GET", "/ws", {**sess, "Upgrade": "websocket", "Connection": "Upgrade",
                                                                                      "Sec-WebSocket-Key": "x", "Sec-WebSocket-Version": "13", "Origin": "http://evil.example"}, 403, "bad_origin"),
            ("POST /fb/api/fs/mkdir with a sign-in, Content-Type: text/plain", "POST", "/fb/api/fs/mkdir", {**sess, "Content-Type": "text/plain"}, 415, "bad_content_type"),
            ("GET /internal/anything over TCP", "GET", "/internal/state", sess, 404, "not_found"),
            ("GET /fb/api/ls without a sign-in", "GET", "/fb/api/ls?path=/tmp", {"Host": host}, 401, "bad_session"),
            ("GET /fb/api/ls with a wrong sign-in", "GET", "/fb/api/ls?path=/tmp", {"Host": host, "X-TM-Session": "0" * 64}, 401, "bad_session"),
        ]
        for name, method, path, headers, status, code in cases:
            st, body, _ = raw(PORT, method, path, headers)
            check(f"AC-09 {name} → {status} {code}", st == status and isinstance(body, dict) and body.get("error") == code, (st, body))
        st, body, _ = raw(PORT, "GET", "/fb/api/ls?path=/tmp", sess)
        check("AC-09 a signed-in request is answered", st == 200 and body.get("status") in ("ready", "loading"), st)
        st, body, _ = raw(PORT, "GET", "/fb/api/raw?path=/etc/hosts&t=" + secret, {"Host": host})
        check("AC-09 an image's GET may carry the sign-in as ?t=", st == 200, st)
        code = unix_request(t.sock, "POST", "/code", {})[1]["code"]
        first = raw(PORT, "GET", f"/open/{code}", {"Host": host})
        again = raw(PORT, "GET", f"/open/{code}", {"Host": host})
        check("AC-09 a code opens the page once; again → 410", first[0] == 200 and again[0] == 410 and "used already" in str(again[1]), (first[0], again[0]))
        check("AC-09 the page with a sign-in is not cached and has a strict CSP", "no-store" in first[2] and "script-src 'self'" in first[2])
        n = "0123456789abcdef0123456789abcdef"
        import hashlib
        sid = hashlib.sha256(secret.encode()).hexdigest()[:16]
        st, body, _ = raw(PORT, "GET", f"/api/hello?n={n}&id={sid}", {"Host": host})
        check("AC-09 the server proves it holds the page's sign-in", st == 200 and body["proof"] == proof(secret, n))
        st, body, _ = raw(PORT, "GET", f"/api/hello?n={n}&id={'f' * 16}", {"Host": host})
        check("AC-09 an unknown sign-in gets no proof", st == 404 and body["error"] == "unknown_signin", (st, body))
        mode = lambda p: stat.S_IMODE(os.stat(p).st_mode)
        check("AC-09 the state folder is 0700, the socket and sign-ins private",
              mode(t.app) == 0o700 and mode(os.path.join(t.app, "local-sessions.json")) == 0o600, oct(mode(t.app)))
        st, body = unix_request(t.sock, "POST", "/signout", {})
        st2, body2, _ = raw(PORT, "GET", "/fb/api/ls?path=/tmp", sess)
        check("AC-09 terminum signout signs every page out", st == 200 and st2 == 401, (st, st2))

        # AC-10 web access
        st, body = unix_request(t.sock, "POST", "/web/on", {})
        check("AC-10 web access needs a password first", st == 409 and "password" in body["message"], body)
        st, body = unix_request(t.sock, "POST", "/web/password", {"password": "short"})
        check("AC-10 a password under 8 characters is refused", st == 409, body)
        cfg = os.path.join(t.app, "web.json")
        st, body = unix_request(t.sock, "POST", "/web/password", {"password": "correct horse"})
        with open(cfg) as f:
            web = json.load(f)
        web["port"], web["host"] = WEB_PORT, "127.0.0.1"   # never on the network during a test
        with open(cfg, "w") as f:
            json.dump(web, f)
        st, body = unix_request(t.sock, "POST", "/web/on", {})
        check("AC-10 web access goes on with its addresses", st == 200 and body["web"]["enabled"], body)
        check("AC-10 web.json holds a salted PBKDF2 hash, 0600", web["password"]["iterations"] >= 100000 and "correct" not in open(cfg).read() and mode(cfg) == 0o600)
        wh = {"Host": f"127.0.0.2:{WEB_PORT}"}
        own = f"localhost:{WEB_PORT}"
        st, body, _ = raw(WEB_PORT, "GET", "/fb/api/ls?path=/tmp", {"Host": own})
        check("AC-10 the network page needs a sign-in", st == 401, st)
        st, body, _ = raw(WEB_PORT, "GET", "/", {"Host": f"evil.example:{WEB_PORT}"})
        check("AC-10 a Host that is not this Mac's is refused", st == 403 and body["error"] == "bad_host", st)
        names = [n for n in t.health()["web"].get("urls", [])]
        lan = names[0].split("//")[1].split(":")[0] if names else socket.gethostname()
        wh = {"Host": f"{lan}:{WEB_PORT}", "Origin": f"http://{lan}:{WEB_PORT}", "Content-Type": "application/json"}
        t0 = time.time()
        st, body, _ = raw(WEB_PORT, "POST", "/auth/login", wh, json.dumps({"password": "wrong pass"}).encode())
        st2, body2, _ = raw(WEB_PORT, "POST", "/auth/login", wh, json.dumps({"password": "wrong again"}).encode())
        check("AC-10 a wrong password is refused and holds the next one", st == 401 and body["reason"] == "wrong" and time.time() - t0 >= 1.9, (st, round(time.time() - t0, 1)))
        st, body, head = raw(WEB_PORT, "POST", "/auth/login", wh, json.dumps({"password": "correct horse"}).encode())
        cookie = next((l.split(":", 1)[1].strip() for l in head.split("\r\n") if l.lower().startswith("set-cookie:")), "")
        check("AC-10 the right password gives an HttpOnly, SameSite=Strict cookie", st == 204 and "HttpOnly" in cookie and "SameSite=Strict" in cookie, cookie[:60])
        token = cookie.split(";")[0]
        st, body, _ = raw(WEB_PORT, "GET", "/fb/api/ls?path=/tmp", {"Host": wh["Host"], "Cookie": token})
        check("AC-10 the cookie signs the network page in", st == 200, st)
        st, body, _ = raw(WEB_PORT, "POST", "/fb/api/terminal/cd", {**wh, "Cookie": token}, b"{}")
        check("AC-10 terminal actions are the Mac's own page's only (not_here)", st == 403 and body["error"] == "not_here", (st, body))
        ws = WS(WEB_PORT, origin=f"http://{lan}:{WEB_PORT}")
        st, body = unix_request(t.sock, "POST", "/web/off", {})
        time.sleep(0.5)
        try:
            refused = socket.create_connection(("127.0.0.1", WEB_PORT), timeout=2) is None
        except OSError:
            refused = True
        check("AC-10 web access off closes the listener", st == 200 and refused, st)
        ws.close()
    finally:
        if not all(results):
            print(t.log())
        t.cleanup()
    summary()


if __name__ == "__main__":
    main()
