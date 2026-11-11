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
const UP: usize = 32;

/// The process at the other end of a connection on the socket; none where the system does not say.
#[derive(Clone, Copy, Debug, Default)]
pub struct Peer(pub Option<u32>);

impl Connected<IncomingStream<'_, UnixListener>> for Peer {
    fn connect_info(stream: IncomingStream<'_, UnixListener>) -> Peer {
        Peer(stream.io().peer_cred().ok().and_then(|c| c.pid()).and_then(|p| u32::try_from(p).ok()))
    }
}

/// Whether `pid` or an ancestor of it is one of `roots`, by one `ps` of every process's parent (off the async workers).
fn descends(pid: u32, roots: &[u32]) -> bool {
    let Ok(out) = Command::new("ps").args(["-A", "-o", "pid=,ppid="]).output() else { return false };
    let parents: std::collections::HashMap<u32, u32> = String::from_utf8_lossy(&out.stdout).lines()
        .filter_map(|l| { let mut f = l.split_whitespace(); Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?)) }).collect();
    let mut at = pid;
    for _ in 0..UP {
        if roots.contains(&at) {
            return true;
        }
        match parents.get(&at) {
            Some(parent) if *parent > 1 => at = *parent,
            _ => return false,
        }
    }
    false
}

/// Whether process `pid` runs under a process of `roots`.
async fn under(pid: u32, roots: Vec<u32>) -> bool {
    tokio::task::spawn_blocking(move || descends(pid, &roots)).await.unwrap_or(false)
}

/// The first processes of `server`'s panes: all of them, or the one of `pane`.
async fn pane_pids(server: &crate::tmux::Server, pane: Option<&str>) -> Vec<u32> {
    let line = match pane {
        Some(p) => format!("display -p -t {p} '#{{pane_pid}}'"),
        None => "list-panes -a -F '#{pane_pid}'".to_string(),
    };
    server.run(line).await.unwrap_or_default().iter().filter_map(|l| l.trim().parse().ok()).collect()
}

/// Whether the call comes from under `pane` of `server`, as it says. An unknown peer is not judged: the socket
/// is the user's own.
pub async fn in_pane(peer: Peer, server: &crate::tmux::Server, pane: &str) -> bool {
    match peer.0 {
        Some(pid) => under(pid, pane_pids(server, Some(pane)).await).await,
        None => {
            tracing::warn!(event = "manager.peer_unknown", pane);
            true
        }
    }
}

/// Whether the call comes from under any pane of this Mac's tmux servers terminum shows.
pub async fn in_any_pane(app: &Shared, peer: Peer) -> bool {
    let Some(pid) = peer.0 else { return false };
    let mut roots = Vec::new();
    for server in app.tmux.all().into_iter().filter(|s| s.is_local()) {
        roots.extend(pane_pids(&server, None).await);
    }
    under(pid, roots).await
}
