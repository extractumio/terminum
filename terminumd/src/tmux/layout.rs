// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The session list (AC-01, AC-39): one `list-panes -a` per server, turned into the page's
//! groups (a tmux session each; a host's group is named after the host) and rows (a pane
//! each, panes after a window's first shifted in under it), with coding agents' states
//! read from their screens' last rows when their title changed or 6 s passed.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::Mutex;
use serde_json::{json, Value};

use super::agentstate::{self, States};
use super::server::{Link, Server, Status, HOST_SESSION};
use super::titles::{self, PaneFields, LIST_PANES_FORMAT};

/// What a server's list read gives: its groups and the pane -> session map.
pub struct Read {
    pub groups: Vec<Value>,
    pub panes: HashMap<String, String>,
    /// every pane key, for workspace expiry (AC-71)
    pub keys: Vec<String>,
}

/// Reads `server`'s panes; `theme(session)` names the theme a session wears (its dot).
pub async fn read(server: &Arc<Server>, states: &Mutex<States>, home: &str, theme: impl Fn(&str) -> String) -> Result<Read, String> {
    let lines = server.run(format!("list-panes -a -F '{LIST_PANES_FORMAT}'")).await?;
    // a host shows only the session terminumd holds there, never the host's others (OQ-81)
    let panes: Vec<PaneFields> = lines.iter().filter_map(|l| titles::parse_list_panes(l))
        .filter(|p| server.is_local() || p.session == HOST_SESSION).collect();
    let agents = agent_states(server, states, &panes).await;
    let mut groups: Vec<Value> = Vec::new();
    let mut map = HashMap::new();
    let mut keys = Vec::new();
    let mut by_session: Vec<(String, Vec<&PaneFields>)> = Vec::new();
    for p in &panes {
        map.insert(p.pane_id.clone(), p.session.clone());
        keys.push(server.key(&p.pane_id));
        match by_session.iter_mut().find(|(s, _)| *s == p.session) {
            Some((_, v)) => v.push(p),
            None => by_session.push((p.session.clone(), vec![p])),
        }
    }
    for (session, mut ps) in by_session {
        ps.sort_by_key(|p| (p.window_index, p.pane_index));
        let profile = theme(&session);
        let mut items = Vec::new();
        let mut i = 0;
        while i < ps.len() {
            let w = ps[i].window_id.clone();
            let n = ps[i..].iter().take_while(|p| p.window_id == w).count();
            for (k, p) in ps[i..i + n].iter().enumerate() {
                let it = titles::item(p, k + 1, n, home);
                let key = server.key(&p.pane_id);
                let state = agents.get(&p.pane_id).copied().flatten();
                items.push(json!({
                    "id": key, "title": it.title, "sub": it.sub, "index": p.window_index.to_string(),
                    "pane": it.pane, "host": (!server.is_local()).then(|| server.name.clone()),
                    "profile": profile, "agent": it.agent, "shell": it.shell && !p.alternate_on,
                    "state": state.map(|s| s.as_str()), "window": p.window_id, "alt": p.alternate_on,
                }));
            }
            i += n;
        }
        let label = if server.is_local() { session.clone() } else { server.name.clone() };
        groups.push(json!({
            "kind": "tmux", "label": label, "session": (!server.is_local()).then(|| session.clone()), "attach": attach_command(server, &session),
            "where": "", "host": (!server.is_local()).then(|| server.name.clone()),
            "wid": format!("{}/{session}", server.name), "new": "New window", "pool": null, "items": items,
        }));
    }
    Ok(Read { groups, panes: map, keys })
}

/// The group a host shows while it is not attached (AC-81): its state and why.
pub fn host_group(server: &Server, files: Option<&str>) -> Value {
    let status = server.status.lock().clone();
    let (state, why) = match &status {
        Status::Attached => ("attached", String::new()),
        Status::Connecting => ("reconnecting", "connecting".to_string()),
        Status::Reconnecting { why, retry_s } => ("reconnecting", format!("reconnecting ({retry_s} s){}", if why.is_empty() { String::new() } else { format!(": {why}") })),
        Status::Failed { why } => ("failed", why.clone()),
        Status::Off => ("off", "off".to_string()),
    };
    json!({"kind": "tmux", "label": server.name, "session": HOST_SESSION, "attach": attach_command(server, HOST_SESSION), "where": "", "host": server.name,
           "wid": format!("{}/{}", server.name, HOST_SESSION),
           "new": "New window", "pool": null, "items": [], "state": state, "why": why, "files": files})
}

/// One word for a POSIX shell: as is when plain, else single-quoted.
fn sh(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "@%_+=:,./-".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', "'\\''"))
    }
}

/// The command that attaches another terminal to `session` (the group's copy button):
/// `tmux -L <socket> attach -t <session>` here, `ssh -t <recipe> 'tmux attach -t terminum'` on a host.
pub fn attach_command(server: &Server, session: &str) -> String {
    let attach = format!("tmux attach -t {}", sh(&format!("={session}")));
    match &server.link {
        Link::Local { socket, .. } => format!("tmux -L {} attach -t {}", sh(socket), sh(&format!("={session}"))),
        Link::Ssh { args, .. } => {
            let recipe: Vec<String> = super::super::hosts::recipe::argv(args).iter().map(|a| sh(a)).collect();
            format!("ssh -t {} {}", recipe.join(" "), sh(&attach))
        }
    }
}

/// Agent states of the panes that run a coding agent, read only when due.
async fn agent_states(server: &Server, states: &Mutex<States>, panes: &[PaneFields]) -> HashMap<String, Option<agentstate::State>> {
    let now = Instant::now();
    let mut out = HashMap::new();
    let mut due = Vec::new();
    for p in panes {
        let is_agent = titles::agent_of(&p.command).is_some() || agentstate::claude_marked(&p.pane_title)
            || titles::AGENTS.contains(&p.window_name.as_str());
        if !is_agent {
            continue;
        }
        let key = server.key(&p.pane_id);
        let g = states.lock();
        if g.due(&key, &p.pane_title, now) {
            due.push(p);
        } else {
            out.insert(p.pane_id.clone(), g.last(&key));
        }
    }
    if due.is_empty() {
        return out;
    }
    let lines: Vec<String> = due.iter().map(|p| format!("capture-pane -p -J -t {}", p.pane_id)).collect();
    let Some(client) = server.bulk() else { return out };
    let answers = client.run_all(lines).await;
    for (p, a) in due.iter().zip(answers) {
        // a pane that cannot be read shows no state this time (AC-39 failure)
        let state = a.ok().map(|rows| agentstate::state_of(&p.pane_title, &rows));
        if let Some(s) = state {
            states.lock().store(&server.key(&p.pane_id), &p.pane_title, now, s);
        }
        out.insert(p.pane_id.clone(), state);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_host_group_says_its_state() {
        let s = Server::new("devbox", "default", Link::Ssh { ssh: "ssh".into(), args: vec!["devbox.example".into()] }, Arc::new(tokio::sync::Notify::new()));
        *s.status.lock() = Status::Reconnecting { why: String::new(), retry_s: 2 };
        let g = host_group(&s, None);
        assert_eq!(g["label"], "devbox");
        assert_eq!(g["state"], "reconnecting");
        assert_eq!(g["why"], "reconnecting (2 s)");
        *s.status.lock() = Status::Failed { why: "Permission denied (publickey)".into() };
        assert_eq!(host_group(&s, Some("enabled"))["why"], "Permission denied (publickey)");
    }

    #[test]
    fn a_group_says_how_another_terminal_attaches() {
        let host = Server::new("devbox", "default", Link::Ssh { ssh: "ssh".into(), args: vec!["-p".into(), "2222".into(), "alex@devbox.example".into()] },
            Arc::new(tokio::sync::Notify::new()));
        assert_eq!(attach_command(&host, HOST_SESSION), "ssh -t -p 2222 -- alex@devbox.example 'tmux attach -t =terminum'");
        assert_eq!(host_group(&host, None)["session"], "terminum");
        let local = Server::new("local", "terminum", Link::Local { tmux: "tmux".into(), socket: "terminum".into(), conf: "/dev/null".into() },
            Arc::new(tokio::sync::Notify::new()));
        assert_eq!(attach_command(&local, "main"), "tmux -L terminum attach -t =main");
        assert_eq!(attach_command(&local, "my work's"), "tmux -L terminum attach -t '=my work'\\''s'");
    }
}
