// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Remote hosts (terminum-remote.md): a host is its record in settings.json (`hosts.<name>`:
//! its checked ssh recipe, files enabled or declined, the helper's build), never a name it
//! reports (AC-87). terminumd holds each host's `terminum` session over ssh as a control
//! client, reconnecting after a drop with back-off 1, 2, 4 … 30 s (AC-81); a host's files go
//! through its agent once enabled (tunnel.rs, AC-82–AC-85).

pub mod recipe;
pub mod tunnel;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use tokio::sync::Notify;

use crate::app::Shared;
use crate::tmux::server::{Link, Server, Status};

/// At most this many hosts (terminum-remote.md limits).
const MOST: usize = 32;
const BACKOFF_MAX: u64 = 30;
/// ssh has this long to reach tmux's first answer.
const ATTACH_WAIT: Duration = Duration::from_secs(20);

/// A host's running parts.
struct Run {
    retry: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
pub struct Hosts {
    runs: Mutex<HashMap<String, Run>>,
    /// a record that cannot be used, by name, and why (AC-87 failure)
    broken: Mutex<HashMap<String, String>>,
}

fn records(app: &Shared) -> Map<String, Value> {
    app.looks.settings.read().hosts.clone()
}

pub(crate) fn ssh_bin() -> std::path::PathBuf {
    crate::config::find_program("ssh", "TM_SSH").unwrap_or_else(|| "/usr/bin/ssh".into())
}

/// Starts, stops or restarts host connections to match settings.json (at start and after
/// every change of the file or a CLI action).
pub fn sync(app: &Shared) {
    let recs = records(app);
    let mut runs = app.hosts.runs.lock();
    let mut broken = HashMap::new();
    for (name, rec) in &recs {
        let args: Vec<String> = rec["ssh"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let checked = recipe::name_ok(name).and_then(|()| recipe::check(&args));
        let args = match checked {
            Ok(a) => a,
            Err(e) => {
                broken.insert(name.clone(), format!("settings.json: {e}"));
                continue;
            }
        };
        let off = rec["off"].as_bool().unwrap_or(false);
        let same = app.tmux.host(name).is_some_and(|s| matches!(&s.link, Link::Ssh { args: a, .. } if *a == args));
        if same && runs.contains_key(name) && !off {
            continue;
        }
        if let Some(r) = runs.remove(name) {
            r.task.abort();
        }
        if let Some(old) = app.tmux.remove_host(name) {
            old.disconnect();
        }
        let server = Server::new(name, "default", Link::Ssh { ssh: ssh_bin(), args }, app.dirty.clone());
        app.tmux.add_host(server.clone());
        if off {
            *server.status.lock() = Status::Off;
            continue;
        }
        let retry = Arc::new(Notify::new());
        let task = tokio::spawn(hold(app.clone(), server, retry.clone()));
        runs.insert(name.clone(), Run { retry, task });
    }
    for name in runs.keys().cloned().collect::<Vec<_>>() {
        if (!recs.contains_key(&name) || broken.contains_key(&name))
            && let Some(r) = runs.remove(&name) {
                r.task.abort();
            }
    }
    for s in app.tmux.hosts() {
        if !recs.contains_key(&s.name) {
            s.disconnect();
            app.tmux.remove_host(&s.name);
            tunnel::stop(app, &s.name);
        }
    }
    *app.hosts.broken.lock() = broken;
    drop(runs);
    app.dirty.notify_one();
}

/// Holds a host's control client: connects, waits while it lives, reconnects with back-off.
async fn hold(app: Shared, server: Arc<Server>, retry: Arc<Notify>) {
    let mut backoff = 1u64;
    loop {
        *server.status.lock() = Status::Connecting;
        app.dirty.notify_one();
        let t0 = std::time::Instant::now();
        let attached = match server.sync_clients(&[]) {
            Ok(()) => wait_attached(&server).await,
            Err(e) => Err(e),
        };
        match attached {
            Ok(()) => {
                tracing::info!(event = "host.attached", host = %server.name, ms = t0.elapsed().as_millis() as u64);
                *server.status.lock() = Status::Attached;
                backoff = 1;
                app.dirty.notify_one();
                tunnel::reconcile(&app, &server.name);
                while server.connected() {
                    tokio::select! {
                        _ = server.ended.notified() => {}
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {}
                    }
                }
                let why = "connection lost".to_string();
                tracing::warn!(event = "host.lost", host = %server.name, retry_s = backoff);
                *server.status.lock() = Status::Reconnecting { why, retry_s: backoff };
            }
            Err(why) => {
                tracing::warn!(event = "host.failed", host = %server.name, why, retry_s = backoff);
                *server.status.lock() = Status::Failed { why };
                server.disconnect();
            }
        }
        app.dirty.notify_one();
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(backoff)) => {}
            _ = retry.notified() => {}
        }
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

/// Until tmux's first answer (attached) or the end of ssh (its last words).
async fn wait_attached(server: &Server) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + ATTACH_WAIT;
    loop {
        if server.attached() {
            return Ok(());
        }
        if !server.connected() {
            return Err(server.last_words().unwrap_or_else(|| "ssh ended".into()));
        }
        if tokio::time::Instant::now() > deadline {
            server.disconnect();
            return Err("no answer from tmux on the host within 20 s".into());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// `hosts add <name> '<destination and options>'` or the page's "Add host": an empty name is
/// the destination's host; `files` records the user's tick to enable its files once attached
/// (AC-83). Returns the host's name.
pub async fn add(app: &Shared, name: &str, text: &str, files: bool) -> Result<String, String> {
    let args = recipe::parse(text)?;
    let recs = records(app);
    let name = if name.is_empty() {
        let made = recipe::default_name(&args)?;
        if recs.get(&made).is_some_and(|r| r["ssh"] != json!(args)) {
            return Err(format!("{made} is already a host: give this one a name"));
        }
        made
    } else {
        recipe::name_ok(name)?;
        name.to_string()
    };
    if recs.len() >= MOST && !recs.contains_key(&name) {
        return Err(format!("At most {MOST} hosts"));
    }
    save(app, |hosts| {
        let mut rec = hosts.get(&name).cloned().unwrap_or_else(|| json!({}));
        rec["ssh"] = json!(args);
        if files && rec["files"] != "enabled" {
            rec["files"] = json!("wanted");
        } else if !files && rec["files"] == "wanted" {
            rec["files"] = Value::Null;
        }
        hosts.insert(name.clone(), rec);
    })?;
    if files {
        tunnel::reconcile(app, &name);
    }
    Ok(name)
}

/// Writes settings.json's hosts and applies them at once (the file watcher would within a second).
fn save(app: &Shared, f: impl FnOnce(&mut Map<String, Value>)) -> Result<(), String> {
    let path = app.cfg.home.join("settings.json");
    crate::settings::save_hosts(&path, f)?;
    let (s, _) = crate::settings::load(&path);
    *app.looks.settings.write() = s;
    sync(app);
    Ok(())
}

/// remove, off, on, retry, enable, decline, files-remove.
pub async fn action(app: &Shared, name: &str, action: &str) -> Result<(), String> {
    if !records(app).contains_key(name) {
        return Err(format!("no host named {name}"));
    }
    match action {
        "remove" => {
            tunnel::stop(app, name);
            save(app, |h| {
                h.remove(name);
            })
        }
        "off" | "on" => save(app, |h| {
            if let Some(r) = h.get_mut(name) {
                r["off"] = json!(action == "off");
            }
        }),
        "retry" => {
            if let Some(r) = app.hosts.runs.lock().get(name) {
                r.retry.notify_one();
            }
            Ok(())
        }
        "enable" => tunnel::enable(app, name).await,
        "decline" => set_files(app, name, Some("declined")),
        "files-remove" => tunnel::remove(app, name).await,
        _ => Err(format!("unknown host action: {action}")),
    }
}

/// Records a host's files state ("enabled", "declined" or none) and its helper's build.
pub fn set_files(app: &Shared, name: &str, files: Option<&str>) -> Result<(), String> {
    save(app, |h| {
        if let Some(r) = h.get_mut(name) {
            r["files"] = json!(files);
        }
    })
}

pub fn set_helper(app: &Shared, name: &str, helper: &str) -> Result<(), String> {
    save(app, |h| {
        if let Some(r) = h.get_mut(name) {
            r["helper"] = json!(helper);
        }
    })
}

/// A host's files state as recorded.
pub fn files_state(app: &Shared, name: &str) -> Option<String> {
    app.looks.settings.read().hosts.get(name).and_then(|r| r["files"].as_str().map(str::to_string))
}

/// The host's ssh recipe (checked).
pub fn recipe_of(app: &Shared, name: &str) -> Option<Vec<String>> {
    match &app.tmux.host(name)?.link {
        Link::Ssh { args, .. } => Some(args.clone()),
        Link::Local { .. } => None,
    }
}

/// `terminum hosts`: each host's record and state (AC-87).
pub fn list(app: &Shared) -> Vec<Value> {
    let broken = app.hosts.broken.lock().clone();
    records(app).iter().map(|(name, rec)| {
        let (state, why) = match (broken.get(name), app.tmux.host(name)) {
            (Some(e), _) => ("failed".to_string(), e.clone()),
            (None, Some(s)) => match s.status.lock().clone() {
                Status::Attached => ("attached".into(), String::new()),
                Status::Connecting => ("reconnecting".into(), "connecting".into()),
                Status::Reconnecting { why, .. } => ("reconnecting".into(), why),
                Status::Failed { why } => ("failed".into(), why),
                Status::Off => ("off".into(), String::new()),
            },
            (None, None) => ("off".into(), String::new()),
        };
        json!({"name": name, "ssh": rec["ssh"], "state": state, "why": why, "files": rec["files"], "helper": rec["helper"],
               "agent": tunnel::state(app, name)})
    }).collect()
}

/// The Files panel's offer for a host's pane (AC-83): ask, dismissed, enabling, connecting,
/// updating, up, down.
pub fn offer(app: &Shared, name: &str) -> Value {
    let state = match files_state(app, name).as_deref() {
        Some("declined") => "dismissed".to_string(),
        Some("enabled") => tunnel::state(app, name),
        // ticked in Add host: set up once attached; a host not reaching it is asked again
        Some("wanted") if app.tmux.host(name).is_some_and(|s| matches!(*s.status.lock(), Status::Attached | Status::Connecting)) => "enabling".into(),
        _ if tunnel::state(app, name) == "enabling" => "enabling".into(),
        _ => "ask".into(),
    };
    let mut v = json!({"key": name, "name": name, "state": state});
    if let Some(why) = tunnel::failed(app, name).filter(|_| state == "ask") {
        v["why"] = json!(why);
    }
    if let Some(b) = tunnel::updated(app, name) {
        v["updated"] = json!(b);
    }
    v
}
