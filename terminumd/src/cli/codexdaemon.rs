// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A shared `codex app-server` (`terminum-agents.md` AC-207): one a Codex app or editor started,
//! listening on a socket, to which a Codex started by hand attaches by itself. It starts that
//! Codex's MCP servers and runs its hooks in its own environment, not the pane's, so terminum
//! cannot tell which pane the agent is in; a TMUX_PANE it has is some other pane's. Under one,
//! `terminum mcp` offers only a tool saying why (mcp.rs) and `terminum hook` sends nothing
//! (hook.rs); `terminum hooks` and its install say when one runs (agentsetup.rs). terminum never
//! stops it and never changes Codex's configuration for it.

use std::sync::LazyLock;

use crate::process::codex::{self, NO_ATTACH, NO_DAEMON, OWN_OPTION, STOP_SERVER};

fn fix() -> String {
    format!("start Codex in the pane with a -c override (codex {OWN_OPTION}) or {NO_DAEMON}; to keep \
plain codex from attaching, run \"{NO_ATTACH}\", then \"{STOP_SERVER}\" (it ends the conversations that server serves)")
}

/// The description of the one tool a session under a shared app-server is offered, and its answer.
pub(super) fn said() -> &'static str {
    static SAID: LazyLock<String> = LazyLock::new(|| format!("terminum cannot see which terminal this Codex runs in: it runs under a shared \
codex app-server (one a Codex app or editor started), which starts terminum's MCP server and hooks without the \
pane's TMUX and TMUX_PANE, so terminum shows no state or plan for it. If this Codex was started in a terminum \
pane, {}. Otherwise go on without terminum's tools.", fix()));
    &SAID
}

/// The one tool a session under a shared app-server is offered: its name says the question.
pub(super) const TOOL: &str = "why_no_terminum_tools";

/// The line `terminum hooks` and its install say when a shared app-server runs.
pub fn warning() -> Option<String> {
    codex::running().map(|pid| format!("Codex: a shared codex app-server runs (process {pid}); a Codex started in a pane \
may attach to it, and terminum then gets no hooks, tools, state or plan from that Codex. To avoid it, {}; \
terminum stops neither.", fix()))
}

#[cfg(test)]
#[path = "codexdaemon_tests.rs"]
mod tests;
