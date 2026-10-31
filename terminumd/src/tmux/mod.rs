// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! tmux: the terminal emulator terminumd is the control client of (AC-01–AC-07). The
//! registry of servers (this Mac's private one, the user's own tmux servers whose sessions
//! are attached by name, AC-89, and the hosts) and pane keys.

pub mod agentstate;
pub mod hookstate;
pub mod args;
pub mod clip;
pub mod command;
pub mod conf;
pub mod control;
pub mod keys;
pub mod layout;
pub mod mouse;
pub mod orphans;
pub mod server;
pub mod titles;

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::RwLock;

pub use server::Server;

/// True the first time `what` asks for this connection (generation) of `server`: work done once
/// a connection, such as giving windows back (`reclaim`) or ending orphans (`orphans`).
pub fn first_on_link(what: &'static str, server: &Server) -> bool {
    static DONE: parking_lot::Mutex<BTreeMap<(&'static str, String), u64>> = parking_lot::const_mutex(BTreeMap::new());
    let generation = server.generation.load(std::sync::atomic::Ordering::Relaxed);
    DONE.lock().insert((what, server.id()), generation) != Some(generation)
}

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
    /// the user's own servers on this Mac, by socket (AC-89)
    attached: RwLock<BTreeMap<String, Arc<Server>>>,
    hosts: RwLock<BTreeMap<String, Arc<Server>>>,
}

impl Tmux {
    pub fn new(local: Arc<Server>) -> Tmux {
        Tmux { local, attached: RwLock::default(), hosts: RwLock::default() }
    }

    /// The servers in list order: this Mac's private one, its attached ones by socket, then
    /// hosts by name.
    pub fn all(&self) -> Vec<Arc<Server>> {
        std::iter::once(self.local.clone()).chain(self.attached.read().values().cloned()).chain(self.hosts.read().values().cloned()).collect()
    }

    pub fn attached(&self, socket: &str) -> Option<Arc<Server>> {
        self.attached.read().get(socket).cloned()
    }

    pub fn attached_all(&self) -> Vec<Arc<Server>> {
        self.attached.read().values().cloned().collect()
    }

    /// The attached server of `socket`, made by `make` when there is none (one per socket).
    pub fn attached_or(&self, socket: &str, make: impl FnOnce() -> Arc<Server>) -> Arc<Server> {
        self.attached.write().entry(socket.to_string()).or_insert_with(make).clone()
    }

    pub fn remove_attached(&self, socket: &str) -> Option<Arc<Server>> {
        self.attached.write().remove(socket)
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
        let server = match host {
            "local" if socket == self.local.socket => self.local.clone(),
            "local" => self.attached(socket)?,
            _ => self.host(host)?,
        };
        (server.socket == socket).then(|| (server, pane.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use server::Link;

    fn local(socket: &str) -> Arc<Server> {
        Server::new("local", socket, Link::Local { tmux: "tmux".into(), socket: socket.into(), conf: "/dev/null".into() }, Default::default())
    }

    /// AC-89: this Mac's servers differ by socket: keys, ids and group ids name the right one,
    /// so their `@N` windows never clash.
    #[test]
    fn an_attached_server_is_told_apart_by_its_socket() {
        let t = Tmux::new(local("terminum"));
        let own = local("default");
        *own.only.lock() = Some(vec!["work".into()]);
        assert!(Arc::ptr_eq(&t.attached_or("default", || own.clone()), &own));
        assert!(Arc::ptr_eq(&t.attached_or("default", || local("default")), &own), "one server per socket");
        assert!(Arc::ptr_eq(&t.resolve("tmux:local:default:%1").unwrap().0, &own));
        assert!(Arc::ptr_eq(&t.resolve("tmux:local:terminum:%1").unwrap().0, &t.local));
        assert!(t.resolve("tmux:local:other:%1").is_none(), "a socket not attached");
        assert_ne!(t.local.id(), own.id());
        assert_eq!((t.local.group_prefix(), own.group_prefix()), ("local".to_string(), "local:default".to_string()));
        assert!(own.is_users() && !t.local.is_users());
        assert!(own.shows("work") && !own.shows("main") && t.local.shows("main"));
        let host = Server::new("devbox", "default", Link::Ssh { ssh: "ssh".into(), args: vec!["devbox.example".into()] }, Default::default());
        assert!(host.shows("terminum") && !host.shows("work"));
        // a host's sessions attached by name (AC-89): shown and the user's; the server is not
        *host.only.lock() = Some(vec!["work".into()]);
        assert!(host.shows("terminum") && host.shows("work") && !host.shows("api"));
        assert!(host.users("work") && !host.users("terminum") && !host.is_users(), "a host is not a server of the user's own");
        assert_eq!((host.group_prefix(), host.label()), ("devbox".to_string(), "devbox".to_string()));
        assert_eq!(t.all().len(), 2);
        assert!(t.remove_attached("default").is_some() && t.resolve("tmux:local:default:%1").is_none());
    }

    #[test]
    fn keys_parse_exactly() {
        assert_eq!(parse_key("tmux:local:terminum:%3"), Some(("local", "terminum", "%3")));
        assert_eq!(parse_key("tmux:devbox:default:%17"), Some(("devbox", "default", "%17")));
        for bad in ["tmux:local:terminum:3", "tmux:local:%3", "local:terminum:%3", "tmux::x:%3", "tmux:a:b:%", "tmux:a:b:%3x"] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
    }
}
