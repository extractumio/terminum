// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;

#[test]
fn keys_name_host_socket_and_pane() {
    let n = Arc::new(Notify::new());
    let s = Server::new("local", "terminum", Link::Local { tmux: "tmux".into(), socket: "terminum".into(), conf: "/dev/null".into() }, n.clone());
    assert_eq!(s.key("%3"), "tmux:local:terminum:%3");
    let h = Server::new("devbox", "default", Link::Ssh { ssh: "ssh".into(), args: vec!["devbox.example".into()] }, n);
    assert_eq!(h.key("%7"), "tmux:devbox:default:%7");
    assert!(!h.is_local());
}

/// Sets the socket path as a connection would have read it (tests only).
pub fn known_socket_path(s: &Server, path: &str) {
    *s.socket_path.lock() = Some((s.generation.load(Ordering::Relaxed), path.to_string()));
}
