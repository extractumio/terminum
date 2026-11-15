#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
# AC-104: sourced by ci_mac.sh before expensive setup; only this run's PATH changes.
ci_mac_node() {
    local bin=${1:-} node="" version="" prefix="" selected=""
    local remedy="set TM_MAC_NODE_BIN to an absolute Node 26 bin directory on the runner; its global Node is unchanged"
    if [ -n "$bin" ]; then
        case "$bin" in
            *:*) echo "ci_mac: Node 26 required: TM_MAC_NODE_BIN cannot contain PATH's colon separator; $remedy" >&2; return 4 ;;
            /*) node="$bin/node" ;;
            *) echo "ci_mac: Node 26 required: TM_MAC_NODE_BIN must be absolute; $remedy" >&2; return 4 ;;
        esac
    else
        node=$(command -v node) || node=""
    fi
    if [ -n "$node" ]; then version=$("$node" --version 2>/dev/null) || version="unavailable"; fi
    case "$version" in v26.*) selected=$node ;; esac
    if [ -z "$selected" ] && [ -z "$bin" ]; then
        prefix=$(brew --prefix node@26 2>/dev/null) || prefix=""
        if [ -n "$prefix" ] && [ -x "$prefix/bin/node" ]; then
            case "$("$prefix/bin/node" --version 2>/dev/null)" in v26.*) selected="$prefix/bin/node" ;; esac
        fi
    fi
    if [ -z "$selected" ]; then
        echo "ci_mac: Node 26 required; found ${version:-no Node} at ${node:-PATH}; $remedy" >&2
        return 4
    fi
    export PATH="$(dirname "$selected"):$PATH"
    echo "ci_mac: using $(node --version) at $(command -v node)"
    node scripts/ci_mac_sqlite.mjs || {
        echo "ci_mac: Node 26 with working node:sqlite numbered parameters required; $remedy" >&2
        return 4
    }
}
ci_mac_node "$@"
