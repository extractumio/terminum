// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! tmux: the terminal emulator terminumd is the control client of (AC-01–AC-07). The
//! registry of servers (this Mac's private one and the attached hosts) and pane keys.

pub mod agentstate;
pub mod args;
pub mod conf;
pub mod control;
pub mod keys;
pub mod layout;
pub mod mouse;
pub mod server;
pub mod titles;

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

pub use server::Server;

/// A pane key's parts: `tmux:<host>:<socket>:%N`.
pub fn parse_key(key: &str) -> Option<(&str, &str, &str)> {
    let rest = key.strip_prefix("tmux:")?;
    let (rest, pane) = rest.rsplit_once(':')?;
    let (host, socket) = rest.split_once(':')?;
    (pane.starts_with('%') && pane[1..].bytes().all(|b| b.is_ascii_digit()) && pane.len() > 1 && !host.is_empty()).then_some((host, socket, pane))
}

/// Every tmux server terminumd attaches to.
pub struct Tmux {
    pub local: Arc<Server>,
    hosts: RwLock<BTreeMap<String, Arc<Server>>>,
}

impl Tmux {
    pub fn new(local: Arc<Server>) -> Tmux {
        Tmux { local, hosts: RwLock::default() }
    }

    /// The servers in list order: this Mac first, then hosts by name.
    pub fn all(&self) -> Vec<Arc<Server>> {
        std::iter::once(self.local.clone()).chain(self.hosts.read().values().cloned()).collect()
    }

    pub fn host(&self, name: &str) -> Option<Arc<Server>> {
        self.hosts.read().get(name).cloned()
    }

    pub fn hosts(&self) -> Vec<Arc<Server>> {
        self.hosts.read().values().cloned().collect()
    }

    pub fn add_host(&self, server: Arc<Server>) {
        self.hosts.write().insert(server.name.clone(), server);
    }

    pub fn remove_host(&self, name: &str) -> Option<Arc<Server>> {
        self.hosts.write().remove(name)
    }

    /// The server and pane id a key names, if that server is attached.
    pub fn resolve(&self, key: &str) -> Option<(Arc<Server>, String)> {
        let (host, socket, pane) = parse_key(key)?;
        let server = if host == "local" { self.local.clone() } else { self.host(host)? };
        (server.socket == socket).then(|| (server, pane.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_exactly() {
        assert_eq!(parse_key("tmux:local:terminum:%3"), Some(("local", "terminum", "%3")));
        assert_eq!(parse_key("tmux:devbox:default:%17"), Some(("devbox", "default", "%17")));
        for bad in ["tmux:local:terminum:3", "tmux:local:%3", "local:terminum:%3", "tmux::x:%3", "tmux:a:b:%", "tmux:a:b:%3x"] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
    }
}
