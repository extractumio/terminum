// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Keeps the tmux side going (AC-01, AC-14): starts the private local server when none runs
//! (or restores the layout after a reboot, AC-15), keeps a control client per session,
//! reads the list on tmux's notices (coalesced 100 ms) and every 2 s while a page looks,
//! tells the pages what changed, says when tmux stops answering, and expires workspaces of
//! panes gone for good (AC-71).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::app::Shared;
use crate::tmux::layout;
use crate::tmux::server::{Server, Status};

/// The list is read this often while a page looks (AC-01).
const EVERY: Duration = Duration::from_secs(2);
/// tmux missing: looked for again this often (AC-01 failure).
const TMUX_AGAIN: Duration = Duration::from_secs(10);
/// A server that has not answered for this long is said to stall (AC-03).
const STALL: Duration = Duration::from_millis(3000);
/// Workspaces of panes gone this long are removed at an inventory this often (AC-71).
const INVENTORY: Duration = Duration::from_secs(300);

/// Starts the local server when none runs, and keeps a control client per session.
async fn ensure_local(app: &Shared) {
    let server = &app.tmux.local;
    // through the bulk client while it lives; a process of its own only when none does
    if server.connected()
        && let Ok(list) = server.run("list-sessions -F '#{session_name}'").await
            && !list.is_empty() {
                if let Err(e) = server.sync_clients(&list) {
                    set_problem(app, Some(e));
                }
                return;
            }
    let sessions = match server.local_sessions().await {
        Ok(s) => s,
        Err(e) => {
            set_problem(app, Some(e));
            return;
        }
    };
    let sessions = match sessions {
        Some(s) if !s.is_empty() => s,
        _ => {
            if crate::restore::at_start(app).await {
                tracing::info!(event = "tmux.restored");
            } else if let Err(e) = server.start_local("main", &crate::config::home().display().to_string()).await {
                set_problem(app, Some(format!("tmux could not start: {e}")));
                return;
            } else {
                tracing::info!(event = "tmux.started", socket = %server.socket);
            }
            match server.local_sessions().await {
                Ok(Some(s)) => s,
                _ => return,
            }
        }
    };
    set_problem(app, None);
    if let Err(e) = server.sync_clients(&sessions) {
        set_problem(app, Some(e));
    } else {
        *server.status.lock() = Status::Attached;
    }
}

fn set_problem(app: &Shared, problem: Option<String>) {
    let mut p = app.tmux_problem.lock();
    if *p != problem {
        *p = problem.clone();
        if let Some(m) = problem {
            tracing::warn!(event = "tmux.problem", msg = %m);
            app.pages.tell(&json!({"t": "error", "msg": m}).to_string());
        }
    }
}

/// One server's groups and pane keys (when `keys` is asked for); false when it could not be read.
async fn read_server(app: &Shared, server: Arc<Server>, home: &str, keys: bool) -> (Vec<Value>, Vec<String>, bool) {
    let local = server.is_local();
    if !local && !server.connected() {
        return (vec![layout::host_group(&server, crate::hosts::files_state(app, &server.name).as_deref())], Vec::new(), false);
    }
    let host = (!local).then(|| server.name.clone());
    let home = if local { home.to_string() } else { server.home().await.unwrap_or_default() };
    let looks = &app.looks;
    let out = match layout::read(&server, &app.agent_states, &home, |s| looks.theme_name(host.as_deref(), s)).await {
        Ok(read) => {
            let mut gs = read.groups;
            if !local {
                let files = crate::hosts::files_state(app, &server.name);
                if gs.is_empty() {
                    gs.push(layout::host_group(&server, files.as_deref()));
                }
                for g in gs.iter_mut().filter(|g| g["state"].is_null()) {
                    g["state"] = json!("attached");
                    g["files"] = json!(files);
                }
            }
            let live = if keys {
                read.keys.iter().map(|k| format!("web:{}", json!([host.clone().unwrap_or_default(), k]))).collect()
            } else {
                Vec::new()
            };
            server.set_panes(read.panes);
            (gs, live, true)
        }
        Err(e) => {
            tracing::warn!(event = "tmux.error", server = %server.name, cmd = "list-panes", msg = e);
            (if local { Vec::new() } else { vec![layout::host_group(&server, None)] }, Vec::new(), false)
        }
    };
    server.prune_wakes();
    out
}

/// Reads every server's list now (all at once: a slow host never holds up the others) and
/// tells the pages when it changed.
pub async fn refresh(app: &Shared) {
    let _one = app.refreshing.lock().await;
    ensure_local(app).await;
    let home = crate::config::home().display().to_string();
    let due = app.inventory_at.lock().is_none_or(|t| t.elapsed() > INVENTORY);
    let reads = futures_util::future::join_all(app.tmux.all().into_iter().map(|s| read_server(app, s, &home, due))).await;
    let complete = reads.iter().all(|(_, _, ok)| *ok);
    let mut groups: Vec<Value> = Vec::new();
    let mut live: HashSet<String> = HashSet::new();
    for (gs, keys, _) in reads {
        groups.extend(gs);
        live.extend(keys);
    }
    {
        let mut last = app.layout.write();
        if last.as_array() != Some(&groups) {
            let told = json!({"t": "layout", "groups": groups}).to_string();
            *last = Value::Array(groups);
            drop(last);
            app.pages.tell(&told);
        }
    }
    if complete && due {
        *app.inventory_at.lock() = Some(Instant::now());
        app.store.maintain(&live, false);
    }
    announce_offers(app);
}

/// A host's files offer changed (its helper's state, its record, its link): its panels are
/// told, so they need not ask again and again (AC-83).
fn announce_offers(app: &Shared) {
    let mut last = app.offers.lock();
    last.retain(|h, _| app.tmux.host(h).is_some());
    for server in app.tmux.hosts() {
        let offer = crate::hosts::offer(app, &server.name);
        if last.get(&server.name) != Some(&offer) {
            last.insert(server.name.clone(), offer);
            app.bus.send(crate::events::Event::Host { host: server.name.clone() });
        }
    }
}

/// The loop: a refresh on every change notice (coalesced), every 2 s while a page looks.
pub async fn run(app: Shared) {
    tokio::spawn(stalls(app.clone()));
    loop {
        refresh(&app).await;
        let wait = if app.tmux_problem.lock().is_some() { TMUX_AGAIN } else if app.pages.any_active() { EVERY } else { Duration::from_secs(30) };
        tokio::select! {
            _ = app.dirty.notified() => tokio::time::sleep(Duration::from_millis(100)).await,
            _ = tokio::time::sleep(wait) => {}
        }
    }
}

/// Says when a server's control client has not answered for 3 s, and when it answers again
/// (the page's stall line, AC-03); the control client itself is restarted after 10 s.
async fn stalls(app: Shared) {
    let mut stalled: HashSet<String> = HashSet::new();
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        for server in app.tmux.all() {
            let waited = server.bulk().map_or(Duration::ZERO, |c| c.waiting());
            let label = server.label().to_string();
            if waited >= STALL && stalled.insert(server.name.clone()) {
                tracing::warn!(event = "tmux.stall", server = %server.name, ms = waited.as_millis() as u64);
                app.pages.tell(&json!({"t": "stall", "server": label, "ok": false, "ms": waited.as_millis() as u64}).to_string());
            } else if waited < STALL && stalled.remove(&server.name) {
                app.pages.tell(&json!({"t": "stall", "server": label, "ok": true}).to_string());
            }
        }
    }
}
