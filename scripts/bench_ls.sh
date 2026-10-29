#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
# bench_ls.sh DIR [RUNS] — AC-52 (terminum-files.md): the cold first page of a large folder
# (not in terminumd's cache) and cached random pages, p95; SORT=recent or ext: in that order. Runs a private terminumd on port
# 47899 with its own home, state and tmux socket, so the installed one is not disturbed.
set -euo pipefail
dir=$(cd "${1:?dir}" && pwd -P); runs=${2:-10}
bin=${TERMINUMD_BIN:-$(dirname "$0")/../terminumd/target/release/terminumd}
home=$(mktemp -d /tmp/tmb-XXXXXX)   # short: the socket path holds at most 104 bytes
tsock=tm-bench-$$
pid=""
trap '[ -n "$pid" ] && kill $pid 2>/dev/null; tmux -L "$tsock" kill-server 2>/dev/null; rm -rf "$home"' EXIT
export TM_PORT=47899 TM_HOME=$home TM_APP_DIR=$home/state TM_TMUX_SOCKET=$tsock TM_LOG=warn
url="http://127.0.0.1:47899/fb/api/ls?path=$dir${SORT:+&sort=$SORT}"
signin() { # a password over the socket (as `terminum password`), then the sign-in /auth/login gives for it
  curl -s -o /dev/null --unix-socket "$TM_APP_DIR/terminumd.sock" -X POST -H 'Content-Type: application/json' \
       -d '{"password":"bench password"}' http://localhost/password
  curl -s -X POST -H 'Content-Type: application/json' -d '{"password":"bench password"}' http://127.0.0.1:47899/auth/login \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["secret"])'
}
cold=(); warm=()
for _ in $(seq "$runs"); do
  "$bin" 2>/dev/null & pid=$!
  for _ in $(seq 100); do curl -s -o /dev/null --unix-socket "$TM_APP_DIR/terminumd.sock" http://localhost/health && break; sleep 0.05; done
  tok=$(signin)
  cold+=("$(curl -s -o /dev/null -w '%{time_total}' -H "X-TM-Session: $tok" "$url&limit=500")")
  total=$(curl -s -H "X-TM-Session: $tok" "$url&limit=1" | python3 -c 'import json,sys;print(json.load(sys.stdin)["total"])')
  [ "$total" -gt 0 ] || { echo "$dir is empty: fill it with scripts/make_big_dir.sh" >&2; exit 1; }
  warm+=("$(curl -s -o /dev/null -w '%{time_total}' -H "X-TM-Session: $tok" "$url&offset=$((RANDOM * RANDOM % total))&limit=500")")
  rss=$(ps -o rss= -p $pid)
  kill $pid; wait $pid 2>/dev/null || true; pid=""
done
p95() { printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END {i=int(NR*0.95); if(i<1)i=1; printf "%d", a[i]*1000}'; }
c=$(p95 "${cold[@]}"); w=$(p95 "${warm[@]}")
verdict=$([ "$c" -lt 2500 ] && [ "$w" -lt 50 ] && [ "$rss" -lt 153600 ] && echo PASS || echo FAIL)
echo "entries=$total cold_p95_ms=$c page_p95_ms=$w rss_mb=$((rss/1024)) $verdict"
