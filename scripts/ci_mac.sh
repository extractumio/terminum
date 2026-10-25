#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
# The macOS checks on a Mac test runner of the owner's network (TM_MAC_RUNNER: its ssh name,
# in local.mk), over ssh, as the owner's Mac pool runs tests: no GitHub runner on a Mac. The tree
# (no builds, dependencies, local files or state) goes to <runner>:~/ci/terminum; there run
# make test, the launchd test under its own label, the Files panel's and the page's browser
# checks (WebKit on macOS is the iPhone's engine). Nothing is installed there: the runner's own
# terminum, if any, is never touched. Its output is evidence, not a gate: run it before a
# release and after macOS-specific changes.
#
#   make test-mac            or   TM_MAC_RUNNER=mac-runner make test-mac
set -euo pipefail
host=${TM_MAC_RUNNER:?"set TM_MAC_RUNNER to the ssh name of the Mac test runner (local.mk: TM_MAC_RUNNER = name)"}
repo=$(cd "$(dirname "$0")/.." && pwd)
ssh -o BatchMode=yes -o ConnectTimeout=10 "$host" 'mkdir -p ci/terminum' \
    || { echo "ci_mac: $host does not answer over ssh (asleep, off, or another network)" >&2; exit 2; }
rsync -a --delete \
    --exclude .git --exclude target --exclude node_modules --exclude ui/dist --exclude dist \
    --exclude .toolchain --exclude 'local.mk' --exclude 'CLAUDE.local.md' --exclude 'MEMORY.md' \
    --exclude '.env*' --exclude 'workspaces.json' --exclude '.claude' --exclude .ci.lock \
    "$repo/" "$host:ci/terminum/"
# one run at a time on the runner (a lock beside the tree); a login shell for Homebrew's PATH
ssh -o BatchMode=yes "$host" 'bash -lc '"'"'
    set -euo pipefail
    cd ~/ci/terminum
    mkdir .ci.lock 2>/dev/null || { echo "another run uses ~/ci/terminum (or a stale ~/ci/terminum/.ci.lock)" >&2; exit 3; }
    trap "rmdir .ci.lock" EXIT
    export PATH="$HOME/.cargo/bin:/opt/homebrew/bin:$PATH" NO_COLOR=1
    # the Rust of the Linux CI, for this run only: the default toolchain of the runner stays as it is
    export RUSTUP_TOOLCHAIN=1.98.1
    rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal >/dev/null
    echo "== $(hostname) $(sw_vers -productVersion), load $(sysctl -n vm.loadavg)"
    make test
    (cd terminumd && TM_LAUNCHD_TEST=1 cargo test --locked --test launchd)
    (cd ui && npx playwright-core install chromium webkit >/dev/null && node test/e2e_panel.mjs)
    (cd terminumd && cargo build --locked)
    node web/test/e2e_all.mjs
    node web/test/e2e_live.mjs
'"'"
