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
use super::hookstate::Hooks;
use super::server::{Link, Server, Status, HOST_SESSION};
use super::titles::{self, PaneFields, LIST_PANES_FORMAT};

/// What a server's list read gives: its groups and the pane -> session map.
pub struct Read {
    pub groups: Vec<Value>,
    pub panes: HashMap<String, String>,
    /// every pane key, for workspace expiry (AC-71)
    pub keys: Vec<String>,
    /// every session the server has, shown or not: name and `$id` (a host's offer, AC-89)
    pub sessions: Vec<(String, String)>,
}

/// Reads `server`'s panes; `theme(session)` names the theme a session wears (its dot).
pub async fn read(server: &Arc<Server>, states: &Mutex<States>, hooks: &Hooks, home: &str, theme: impl Fn(&str) -> String) -> Result<Read, String> {
    let lines = server.run(format!("list-panes -a -F '{LIST_PANES_FORMAT}'")).await?;
    let all: Vec<PaneFields> = lines.iter().filter_map(|l| titles::parse_list_panes(l)).collect();
    let mut sessions: Vec<(String, String)> = Vec::new();
    for p in &all {
        if !sessions.iter().any(|(n, _)| *n == p.session) {
            sessions.push((p.session.clone(), p.session_id.clone()));
        }
    }
    // a host shows the session terminumd holds there and those the user attached, never the
    // host's others (OQ-81, AC-89)
    let panes: Vec<PaneFields> = all.into_iter().filter(|p| server.shows(&p.session)).collect();
    let (agents, views) = agent_states(server, states, hooks, &panes).await;
    hooks.retain(&server.key(""), &panes.iter().map(|p| server.key(&p.pane_id)).collect::<Vec<_>>());
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
        let mut panes_of = Vec::new();
        let mut i = 0;
        while i < ps.len() {
            let w = ps[i].window_id.clone();
            let n = ps[i..].iter().take_while(|p| p.window_id == w).count();
            for (k, p) in ps[i..i + n].iter().enumerate() {
                let it = titles::item(p, k + 1, n, home);
                let key = server.key(&p.pane_id);
                let state = agents.get(&p.pane_id).copied().flatten();
                // the hooks say what runs and does (AC-39), else the program's name and the screen
                let hook = views.get(&p.pane_id);
                let agent = it.agent.or(hook.as_ref().map(|h| h.agent)).or_else(|| agents.contains_key(&p.pane_id).then_some("codex"));
                let mut row = json!({
                    "id": key, "title": it.title, "sub": it.sub, "index": p.window_index.to_string(), "dir": p.path,
                    "pane": it.pane, "host": (!server.is_local()).then(|| server.name.clone()),
                    "profile": profile, "agent": agent, "shell": it.shell && !p.alternate_on,
                    "state": state.map(|s| s.as_str()), "window": p.window_id, "alt": p.alternate_on,
                });
                if let Some(h) = hook.filter(|h| h.state != agentstate::State::Done) {
                    for (k, v) in [("tool", &h.tool), ("why", &h.why)] {
                        if !v.is_empty() { row[k] = json!(v); }
                    }
                    if h.subagents > 0 { row["subagents"] = json!(h.subagents); }
                }
                items.push(row);
                panes_of.push(*p);
            }
            i += n;
        }
        let items = quick_rows(server, items, &panes_of);
        let users = server.users(&session);
        let label = if server.is_local() || users { session.clone() } else { server.name.clone() };
        let mut g = json!({
            "kind": "tmux", "label": label, "session": (!server.is_local()).then(|| session.clone()), "attach": attach_command(server, &session),
            "where": "", "host": (!server.is_local()).then(|| server.name.clone()), "server": server.id(),
            "wid": format!("{}/{session}", server.group_prefix()), "new": "New window", "pool": null, "items": items,
        });
        if users {
            attached_fields(&mut g, server);
        }
        groups.push(g);
    }
    Ok(Read { groups, panes: map, keys, sessions })
}

/// Quick shells (AC-88): the rows of a window marked `@tm-quick %N`, whose pane %N is in the
/// same session and not in a quick shell itself, get `quickOf` (the page lists them under
/// %N's), and %N's row names the first such window (`quick`, `quickSide`). Any other mark (the
/// pane gone, a second quick shell) leaves an ordinary window. Rows keep tmux's order.
fn quick_rows(server: &Server, mut items: Vec<Value>, panes: &[&PaneFields]) -> Vec<Value> {
    let mut of_window: HashMap<&str, usize> = HashMap::new();     // quick window -> its parent row
    let mut has: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for p in panes {
        if p.quick_for.is_empty() || of_window.contains_key(p.window_id.as_str()) {
            continue;
        }
        let parent = panes.iter().position(|x| x.pane_id == p.quick_for && x.window_id != p.window_id && x.quick_for.is_empty());
        if let Some(i) = parent.filter(|i| !has.contains(i)) {
            has.insert(i);
            of_window.insert(p.window_id.as_str(), i);
            items[i]["quick"] = json!(server.key(&p.pane_id));
            items[i]["quickSide"] = json!(match p.quick_side.as_str() { "left" => "left", "right" => "right", _ => "bottom" });
        }
    }
    for (row, p) in items.iter_mut().zip(panes) {
        if let Some(&i) = of_window.get(p.window_id.as_str()) {
            row["quickOf"] = json!(server.key(&panes[i].pane_id));
            if let Some(sub) = row["sub"].as_array_mut() {
                sub.insert(0, json!("quick shell"));
            }
        }
    }
    items
}

/// An attached session's group (AC-89): which of the user's tmux servers it is on (the page
/// tells them apart by `server`); its label is its name, so no session chip.
fn attached_fields(g: &mut Value, server: &Server) {
    g["where"] = json!(if server.socket == "default" { "tmux".to_string() } else { format!("tmux -L {}", server.socket) });
    g["attached"] = json!(server.socket);
    g["session"] = Value::Null;
}

/// The group of an attached session that does not run (AC-89): it attaches when it appears.
pub fn absent_group(server: &Server, session: &str, why: &str) -> Value {
    let host = (!server.is_local()).then(|| server.name.clone());
    let mut g = json!({"kind": "tmux", "label": session, "session": null, "attach": attach_command(server, session), "host": host,
        "server": server.id(), "wid": format!("{}/{session}", server.group_prefix()), "new": null, "pool": null, "items": [],
        "state": "absent", "why": why});
    attached_fields(&mut g, server);
    g
}

/// The group a host shows while it is not attached (AC-81): its state and why; no new window
/// while there is no tmux to make it in.
pub fn host_group(server: &Server, files: Option<&str>) -> Value {
    let status = server.status.lock().clone();
    let (state, why) = match &status {
        Status::Attached => ("attached", String::new()),
        Status::Connecting => ("reconnecting", "connecting".to_string()),
        Status::Reconnecting { why, retry_s } => ("reconnecting", format!("reconnecting ({retry_s} s){}", if why.is_empty() { String::new() } else { format!(": {why}") })),
        Status::Failed { why } => ("failed", why.clone()),
        Status::Off => ("off", "off".to_string()),
    };
    json!({"kind": "tmux", "label": server.name, "session": HOST_SESSION, "attach": attach_command(server, HOST_SESSION), "where": "", "host": server.name, "server": server.id(),
           "wid": format!("{}/{}", server.name, HOST_SESSION),
           "new": (state == "attached").then_some("New window"), "pool": null, "items": [], "state": state, "why": why, "files": files})
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
        Link::Local { socket, .. } if socket == "default" => attach,
        Link::Local { socket, .. } => format!("tmux -L {} attach -t {}", sh(socket), sh(&format!("={session}"))),
        Link::Ssh { args, .. } => {
            let recipe: Vec<String> = super::super::hosts::recipe::argv(args).iter().map(|a| sh(a)).collect();
            format!("ssh -t {} {}", recipe.join(" "), sh(&attach))
        }
    }
}

/// Whether pane `p` runs a coding agent by its program, title or window's name.
fn named_agent(p: &PaneFields) -> bool {
    titles::agent_of(&p.command).is_some() || agentstate::claude_marked(&p.pane_title)
        || titles::AGENTS.contains(&p.window_name.as_str())
}

/// Whether pane `p` may run Codex installed with npm (its program is `node`): its screen says.
fn maybe_codex(p: &PaneFields) -> bool {
    p.command.trim() == "node"
}

/// The hooks' state while it holds (AC-39), no screen read for that pane; but a wait they said
/// gives way to a screen that shows the agent at work (an approval answered fires no event until
/// its tool ends).
/// Also the hooks' views, by pane, for the rows.
async fn agent_states(server: &Server, states: &Mutex<States>, hooks: &Hooks, panes: &[PaneFields])
    -> (HashMap<String, Option<agentstate::State>>, HashMap<String, super::hookstate::View>) {
    use agentstate::State::{Waiting, Working};
    let now = Instant::now();
    let (mut held, mut read) = (HashMap::new(), Vec::new());
    for p in panes {
        match hooks.view(&server.key(&p.pane_id), &p.command, now) {
            Some(h) => {
                if h.state == Waiting {
                    read.push(p);
                }
                held.insert(p.pane_id.clone(), h);
            }
            None => read.push(p),
        }
    }
    let mut out = screen_states(server, states, &read, now).await;
    for (id, h) in &held {
        if !(h.state == Waiting && out.get(id) == Some(&Some(Working))) {
            out.insert(id.clone(), Some(h.state));
        }
    }
    (out, held)
}

/// Agent states of the panes that run a coding agent, read only when due; a `node` pane is one
/// while its last read screen is Codex's (its state is `None` otherwise, as for any program).
async fn screen_states(server: &Server, states: &Mutex<States>, panes: &[&PaneFields], now: Instant) -> HashMap<String, Option<agentstate::State>> {
    let mut out = HashMap::new();
    let mut due = Vec::new();
    for &p in panes {
        let named = named_agent(p);
        if !named && !maybe_codex(p) {
            continue;
        }
        let key = server.key(&p.pane_id);
        let g = states.lock();
        if g.due(&key, &p.pane_title, now) {
            due.push(p);
        } else if named || g.codex(&key) {
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
        let key = server.key(&p.pane_id);
        let rows = a.ok();
        let mut g = states.lock();
        let codex = !named_agent(p) && rows.as_deref().map_or(g.codex(&key), agentstate::codex_screen);
        // a pane that cannot be read shows no state this time (AC-39 failure)
        let state = rows.map(|rows| agentstate::state_of(&p.pane_title, &rows));
        if let Some(s) = state {
            g.store(&key, &p.pane_title, now, s, codex);
        }
        if named_agent(p) || codex {
            out.insert(p.pane_id.clone(), state);
        }
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

    /// AC-88: rows are only tagged, in tmux's order; a quick shell of a quick shell, a second
    /// one of a pane, one whose pane is gone, stay ordinary windows.
    #[test]
    fn quick_shells_are_tagged_under_their_pane() {
        let s = Server::new("local", "terminum", Link::Local { tmux: "tmux".into(), socket: "terminum".into(), conf: "/dev/null".into() },
            Arc::new(tokio::sync::Notify::new()));
        let pane = |id: &str, w: &str, of: &str| PaneFields { pane_id: id.into(), window_id: w.into(), quick_for: of.into(), quick_side: "left".into(), ..Default::default() };
        let ps = [pane("%1", "@1", ""), pane("%2", "@2", "%1"), pane("%3", "@3", "%2"), pane("%4", "@4", "%1"), pane("%5", "@5", "%9")];
        let rows: Vec<Value> = ps.iter().map(|p| json!({"id": s.key(&p.pane_id), "sub": ["zsh"]})).collect();
        let refs: Vec<&PaneFields> = ps.iter().collect();
        let out = quick_rows(&s, rows, &refs);
        let ids: Vec<&str> = out.iter().map(|r| r["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["tmux:local:terminum:%1", "tmux:local:terminum:%2", "tmux:local:terminum:%3", "tmux:local:terminum:%4", "tmux:local:terminum:%5"]);
        assert_eq!((out[0]["quick"].as_str(), out[0]["quickSide"].as_str()), (Some("tmux:local:terminum:%2"), Some("left")));
        assert_eq!(out[1]["quickOf"], "tmux:local:terminum:%1");
        assert_eq!(out[1]["sub"], json!(["quick shell", "zsh"]));
        for (i, why) in [(2, "a quick shell's own"), (3, "a second one"), (4, "its pane gone")] {
            assert!(out[i].get("quickOf").is_none() && out[i].get("quick").is_none(), "{why}");
        }
        assert!(out[1].get("quick").is_none(), "a quick shell is never a parent");
    }
}
