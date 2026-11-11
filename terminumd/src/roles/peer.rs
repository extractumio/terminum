// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Which process calls on the private socket (`terminum-agents.md` AC-178): the system's word for the peer's pid,
//! and its ancestors by `ps`. A manager's call must come from under the pane it names; minting a token must come
//! from outside every pane. A tripwire for an agent that names another pane, no boundary: a process of the same
//! user can leave its ancestry, or reach what the tools reach without them.

use std::process::Command;

use axum::extract::connect_info::Connected;
use axum::serve::IncomingStream;
use tokio::net::UnixListener;

use crate::app::Shared;

/// The ancestors looked at: a tool's call sits a few programs under its pane's first.
const UP: usize = 24;

/// The process at the other end of a connection on the socket; none where the system does not say.
#[derive(Clone, Copy, Debug, Default)]
pub struct Peer(pub Option<u32>);

impl Connected<IncomingStream<'_, UnixListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Peer {
        Peer(stream.io().peer_cred().ok().and_then(|c| c.pid()).and_then(|p| u32::try_from(p).ok()))
    }
}

/// `pid` and its ancestors, nearest first, up to the first process (off the async workers: one `ps` each).
fn ancestry(pid: u32) -> Vec<u32> {
    let mut chain = vec![pid];
    let mut at = pid;
    for _ in 0..UP {
        let Ok(out) = Command::new("ps").args(["-o", "ppid=", "-p", &at.to_string()]).output() else { break };
        let Some(parent) = String::from_utf8_lossy(&out.stdout).trim().parse::<u32>().ok().filter(|p| *p > 1) else { break };
        chain.push(parent);
        at = parent;
    }
    chain
}

/// Whether `peer` runs under a process of `roots`. An unknown peer is not judged: the socket is the user's own.
async fn under(peer: Peer, roots: Vec<u32>) -> bool {
    let Some(pid) = peer.0 else { return true };
    tokio::task::spawn_blocking(move || ancestry(pid).iter().any(|p| roots.contains(p))).await.unwrap_or(false)
}

/// The first processes of `server`'s panes: all of them, or the one of `pane`.
async fn pane_pids(server: &crate::tmux::Server, pane: Option<&str>) -> Vec<u32> {
    let line = match pane {
        Some(p) => format!("display -p -t {p} '#{{pane_pid}}'"),
        None => "list-panes -a -F '#{pane_pid}'".to_string(),
    };
    server.run(line).await.unwrap_or_default().iter().filter_map(|l| l.trim().parse().ok()).collect()
}

/// Whether the call comes from under `pane` of `server`, as it says.
pub async fn in_pane(peer: Peer, server: &crate::tmux::Server, pane: &str) -> bool {
    peer.0.is_none() || under(peer, pane_pids(server, Some(pane)).await).await
}

/// Whether the call comes from under any pane of this Mac's tmux servers terminum shows.
pub async fn in_any_pane(app: &Shared, peer: Peer) -> bool {
    let Some(_) = peer.0 else { return false };
    let mut roots = Vec::new();
    for server in app.tmux.all().into_iter().filter(|s| s.is_local()) {
        roots.extend(pane_pids(&server, None).await);
    }
    under(peer, roots).await
}
