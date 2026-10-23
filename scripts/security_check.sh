#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
# security_check.sh — CLAUDE.md §2 and AC-09, in two parts:
#  1. The tree (tracked and new files, not ignored ones): no secrets, home paths, personal
#     names or private hosts. What is personal comes from this machine when it runs (login
#     name, full name, git author and email, home folder, host name) and from the file named
#     by TM_PRIVATE_PATTERNS (one regular expression a line, kept outside the repository), so
#     this script holds none of it. The copyright holder's name and email may appear in
#     LICENSE-COMMERCIAL.md, CONTRIBUTING.md, README.md, Cargo.toml and package.json.
#     Findings name the file, line and kind, never the text found.
#  2. With TM_PORT and TM_APP_DIR of a private terminumd (never the live one): its refusals.
set -uo pipefail
cd "$(dirname "$0")/.." || exit 1
fail=0

full_name=$(id -F 2>/dev/null || true)
host_names="$(scutil --get LocalHostName 2>/dev/null || true) $(hostname -s 2>/dev/null || true)"
TM_SC_LOGIN=$(id -un) TM_SC_FULL="$full_name" TM_SC_HOSTS="$host_names" \
TM_SC_GIT_NAME=$(git config user.name 2>/dev/null || true) TM_SC_GIT_EMAIL=$(git config user.email 2>/dev/null || true) \
python3 - <<'PY' || fail=1
import os, re, subprocess, sys

files = subprocess.run(["git", "ls-files", "-co", "--exclude-standard", "-z"], capture_output=True).stdout.decode().split("\0")
ALLOWED = {"LICENSE-COMMERCIAL.md", "CONTRIBUTING.md", "README.md", "Cargo.toml", "package.json"}
SECRETS = [
    ("Anthropic key", r"sk-ant-[A-Za-z0-9_-]{16,}"), ("API key", r"\bsk-[A-Za-z0-9]{32,}"),
    ("AWS key", r"\bAKIA[0-9A-Z]{16}\b"), ("GitHub token", r"\bgh[pousr]_[A-Za-z0-9]{30,}"),
    ("GitHub token", r"github_pat_[A-Za-z0-9_]{20,}"), ("GitLab token", r"glpat-[A-Za-z0-9_-]{20,}"),
    ("Slack token", r"xox[abpsr]-[A-Za-z0-9-]{10,}"), ("private key", r"-----BEGIN [A-Z ]*PRIVATE KEY-----"),
    ("JWT", r"\beyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\."),
    # example users are alex… (CLAUDE.md §2); /Users/Shared is no one's home
    ("home path", r"/Users/(?!alex|Shared\b)[A-Za-z][\w.-]*"), ("home path", r"/home/(?!alex)[a-z][\w.-]{2,}"),
]
home = os.path.expanduser("~")
if home not in ("/", "") and not re.fullmatch(r"/(Users|home)/alex\w*", home):
    SECRETS.append(("home path", re.escape(home) + r"\b"))
for name in os.environ.get("TM_SC_HOSTS", "").split():
    if len(name) >= 4:
        SECRETS.append(("this Mac's name", r"(?i)\b" + re.escape(name) + r"\b"))
extra = os.environ.get("TM_PRIVATE_PATTERNS")
if extra:
    with open(extra) as f:
        SECRETS += [("private pattern", line.strip()) for line in f if line.strip() and not line.startswith("#")]
PERSONAL = []
for kind, value in (("login name", "TM_SC_LOGIN"), ("full name", "TM_SC_FULL"), ("git author", "TM_SC_GIT_NAME"),
                    ("git email", "TM_SC_GIT_EMAIL")):
    v = os.environ.get(value, "").strip()
    if len(v) >= 4 and v != "alex":
        PERSONAL.append((kind, r"(?<![\w.])" + re.escape(v) + r"(?![\w])"))
rules = [(k, re.compile(p)) for k, p in SECRETS]
personal = [(k, re.compile(p)) for k, p in PERSONAL]
found = 0
for path in filter(None, files):
    if os.path.islink(path) or not os.path.isfile(path) or os.path.getsize(path) > 4 << 20:
        continue
    data = open(path, "rb").read()
    if b"\0" in data:
        continue
    checks = rules + ([] if os.path.basename(path) in ALLOWED else personal)
    for n, line in enumerate(data.decode("utf-8", "replace").splitlines(), 1):
        for kind, rx in checks:
            if rx.search(line):
                print(f"FAIL {path}:{n}: {kind}")
                found += 1
print(f"{'FAIL' if found else 'ok  '} tree: {len([f for f in files if f])} files, {found} findings")
sys.exit(1 if found else 0)
PY

if [ -n "${TM_PORT:-}" ] && [ -n "${TM_APP_DIR:-}" ]; then
  app=$TM_APP_DIR; b="http://127.0.0.1:$TM_PORT"; sock="$app/terminumd.sock"
  [ "$TM_PORT" != 48080 ] || { echo "Refusing port 48080, the live terminumd's: run it against a private one" >&2; exit 1; }
  if python3 -c 'import os,sys; from pathlib import Path; sys.exit(Path(sys.argv[1]).resolve() == (Path.home() / ".terminum/state").resolve())' "$app"; then :; else
    echo "Refusing the live state folder: run it against a private terminumd (TM_APP_DIR=<its short folder>)" >&2; exit 1
  fi
  code() { curl -s -o /dev/null -w '%{http_code}' -m 5 "$@"; }
  check() { # name expected actual
    if [ "$2" = "$3" ]; then echo "ok   $1 → $3"; else echo "FAIL $1 → $3 (want $2)"; fail=1; fi; }
  # a password set over the socket (as `terminum password`), then the sign-in /auth/login gives (never printed)
  pw='security check password'
  curl -s -o /dev/null -m 5 --unix-socket "$sock" -X POST -H 'Content-Type: application/json' -d "{\"password\":\"$pw\"}" http://localhost/password
  secret=$(curl -s -m 5 -X POST -H 'Content-Type: application/json' -d "{\"password\":\"$pw\"}" "$b/auth/login" \
           | python3 -c 'import json,sys; print(json.load(sys.stdin).get("secret", ""))' 2>/dev/null)
  [ -n "$secret" ] || { echo "FAIL no sign-in from the password (POST /password on the socket, then POST /auth/login)"; exit 1; }
  ws=(-H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==')
  check "wrong password"            401 "$(code -X POST -H 'Content-Type: application/json' -d '{"password":"wrong one"}' "$b/auth/login")"
  check "tailscale funnel"          403 "$(code -H 'Tailscale-Funnel-Request: ?1' -H "X-TM-Session: $secret" "$b/fb/api/state")"
  check "forwarded: Finder"         403 "$(code -X POST -H 'X-Forwarded-Host: mac.tail1234.ts.net' -H 'Content-Type: application/json' -H "X-TM-Session: $secret" -d '{}' "$b/fb/api/os/reveal")"
  check "no sign-in: /ws"           401 "$(code "${ws[@]}" "$b/ws")"
  check "no sign-in: files"         401 "$(code "$b/fb/api/state")"
  check "wrong sign-in"             401 "$(code -H 'X-TM-Session: nope' "$b/fb/api/state")"
  check "bad host"                  403 "$(code -H "Host: evil.example:$TM_PORT" -H "X-TM-Session: $secret" "$b/fb/api/state")"
  check "bad origin: /ws"           403 "$(code "${ws[@]}" -H 'Origin: http://evil.example' -H "Sec-WebSocket-Protocol: tm, $secret" "$b/ws")"
  check "bad origin: write"         403 "$(code -X POST -H 'Origin: http://evil.example' -H 'Content-Type: application/json' -H "X-TM-Session: $secret" -d '{}' "$b/fb/api/fs/mkdir")"
  check "bad content type"          415 "$(code -X POST -H 'Content-Type: text/plain' -H "X-TM-Session: $secret" -d '{}' "$b/fb/api/fs/mkdir")"
  check "sign-in in query on write" 401 "$(code -X PUT -H 'Content-Type: application/json' -d '{}' "$b/fb/api/prefs?t=$secret")"
  check "sign-in in query on read"  401 "$(code "$b/fb/api/raw?path=/etc/hosts&t=$secret")"
  check "internal over TCP"         404 "$(code -H "X-TM-Session: $secret" "$b/internal/state")"
  check "socket routes over TCP"    404 "$(code -X POST -H 'Content-Type: application/json' -H "X-TM-Session: $secret" -d '{"password":"x"}' "$b/password")"
  sid=$(printf %s "$secret" | shasum -a 256 | cut -c1-16)
  check "hello needs no sign-in"    200 "$(code "$b/api/hello?n=00112233445566778899aabbccddeeff&id=$sid")"
  check "device file"               400 "$(code -H "X-TM-Session: $secret" "$b/fb/api/raw?path=/dev/zero")"
  check "relative path"             400 "$(code -H "X-TM-Session: $secret" "$b/fb/api/file?path=../../etc/passwd")"
  check "socket: health"            200 "$(code --unix-socket "$sock" http://localhost/health)"
  check "private state folder"      700 "$(stat -f %Lp "$app")"
  check "private socket"            600 "$(stat -f %Lp "$sock")"
  check "valid request"             200 "$(code -H "X-TM-Session: $secret" "$b/fb/api/state")"
else
  echo "skip terminumd refusals: set TM_PORT and TM_APP_DIR of a private terminumd to run them"
fi
[ $fail = 0 ] && echo PASS || { echo FAIL; exit 1; }
