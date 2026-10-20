// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The layout survives a reboot and a killed tmux (AC-15). terminumd writes `state/layout.json`
//! within 5 s of a change: sessions, windows (name, `#{window_layout}`), panes (folder, the
//! program's kind: shell, an ssh recipe, other), the boot it was seen in, and the other servers
//! it shows (restore_srv.rs: hosts, the user's own tmux). When terminum's tmux has no session
//! (after a reboot, or its server was killed), terminumd recreates the sessions with shells in
//! their folders, applies each window's layout and writes a report the first page shows. Never
//! stored: output, history, environment, command lines other than a checked ssh recipe.

use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::tmux::server::Link;

const EVERY: Duration = Duration::from_secs(5);
use crate::restore_srv::{make, sessions_of, Exec, Made, PANES, WINDOWS};

#[derive(Default)]
pub struct Restore {
    /// the last checkpoint written (its JSON), to write only changes
    last: Mutex<String>,
    /// the report of the last restore, until a page has shown it
    pub report: Mutex<Option<Value>>,
    /// the servers (and a host's tmux start) made again while terminumd runs: once each
    tried: Mutex<std::collections::HashSet<String>>,
}

impl Restore {
    /// Whether `what` is tried for the first time (then it is not again).
    pub fn first_try(&self, what: &str) -> bool {
        self.tried.lock().insert(what.to_string())
    }

    /// Whether `what` was tried already.
    pub fn was_tried(&self, what: &str) -> bool {
        self.tried.lock().contains(what)
    }
}

/// This boot's id: macOS's boot time, Linux's boot id (TM_BOOT_ID plays another boot in tests).
/// Read once: it cannot change while terminumd runs.
fn boot_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        if let Ok(id) = std::env::var("TM_BOOT_ID") {
            return id;
        }
        if let Ok(id) = std::fs::read_to_string("/proc/sys/kernel/random/boot_id") {
            return id.trim().to_string();
        }
        std::process::Command::new("/usr/sbin/sysctl").args(["-n", "kern.boottime"]).output().ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).split(',').next().unwrap_or("").trim().to_string())
            .unwrap_or_default()
    })
}

fn enabled(app: &Shared) -> bool {
    app.looks.settings.read().restore
}

fn path(app: &Shared) -> std::path::PathBuf {
    app.cfg.dir.join("layout.json")
}

/// The processes on the terminals of panes running ssh, by terminal (`ttys001`): one `ps`;
/// one a terminal when that fails (`ps` prints nothing when one of them is gone).
async fn ssh_commands(ttys: &[&str]) -> Vec<(String, Vec<String>)> {
    async fn ps(list: &str) -> Option<Vec<(String, Vec<String>)>> {
        let o = tokio::process::Command::new("/bin/ps").args(["-o", "tty=,args=", "-t", list]).output().await.ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).lines().filter_map(|line| {
            let mut words = line.split_whitespace().map(str::to_string);
            Some((words.next()?, words.collect()))
        }).collect())
    }
    let short: Vec<&str> = ttys.iter().map(|t| t.trim_start_matches("/dev/")).collect();
    if short.is_empty() {
        return Vec::new();
    }
    if let Some(all) = ps(&short.join(",")).await {
        return all;
    }
    let mut out = Vec::new();
    for t in short {
        out.extend(ps(t).await.unwrap_or_default());
    }
    out
}

/// A pane's kind: a shell, a checked ssh recipe (read from the pane's terminal), or other.
fn kind(command: &str, tty: &str, procs: &[(String, Vec<String>)]) -> Value {
    if crate::tmux::titles::SHELLS.contains(&command.trim_start_matches('-')) {
        return json!({"kind": "shell"});
    }
    if command == "ssh" && tty.starts_with("/dev/") {
        let short = tty.trim_start_matches("/dev/");
        for (_, words) in procs.iter().filter(|(t, _)| short.ends_with(t.as_str())) {
            if words.first().is_some_and(|w| w.rsplit('/').next() == Some("ssh"))
                && let Ok(recipe) = crate::hosts::recipe::check(&words[1..]) {
                    return json!({"kind": "ssh", "recipe": recipe});
                }
        }
    }
    json!({"kind": "other", "program": command})
}

/// The local server's layout as layout.json holds it.
async fn checkpoint(app: &Shared, prev: &Value) -> Option<Value> {
    let mut answers = app.tmux.local.bulk()?.run_all(vec![format!("list-windows -a -F '{WINDOWS}'"), format!("list-panes -a -F '{PANES}'")]).await.into_iter();
    let (wins, panes) = (answers.next()?.ok()?, answers.next()?.ok()?);
    let ssh_ttys: Vec<&str> = panes.iter().map(|l| l.splitn(6, '\t').collect::<Vec<_>>()).filter(|g| g.len() == 6 && g[3] == "ssh").map(|g| g[4]).collect();
    let procs = ssh_commands(&ssh_ttys).await;
    let sessions = sessions_of(&wins, &panes, |_| true, |c, tty| kind(c, tty, &procs));
    let hosts: Vec<String> = app.tmux.hosts().iter().map(|h| h.name.clone()).collect();
    let servers = crate::restore_srv::checkpoint(app, &prev["servers"]).await;
    Some(json!({"version": 2, "boot": boot_id(), "sessions": sessions, "hosts": hosts, "servers": servers}))
}

/// Writes the checkpoint within 5 s of a change, while restore is on.
pub async fn follow(app: Shared) {
    loop {
        tokio::time::sleep(EVERY).await;
        if !enabled(&app) || app.tmux_problem.lock().is_some() {
            continue;
        }
        let prev = std::fs::read_to_string(path(&app)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or(Value::Null);
        let Some(mut v) = checkpoint(&app, &prev).await else { continue };
        // a change of the layout writes, not the time a server was read
        let mut same = v.clone();
        for s in same["servers"].as_object_mut().into_iter().flat_map(|m| m.values_mut()) {
            s.as_object_mut().map(|o| o.remove("at"));
        }
        let text = same.to_string();
        if *app.restore.last.lock() == text {
            continue;
        }
        *app.restore.last.lock() = text;
        v["at"] = json!(crate::auth::store::unix_now());
        if let Err(e) = crate::config::write_private(&path(&app), v.to_string().as_bytes()) {
            tracing::warn!(event = "restore.checkpoint_failed", error = %e);
        }
    }
}

/// terminum's tmux has no session (a reboot, its server killed, its last session closed while
/// terminumd ran, but not by the user closing its last pane): the last checkpoint is made again. True when sessions were made.
pub async fn at_start(app: &Shared) -> bool {
    if !enabled(app) {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(path(app)) else { return false };
    let Ok(saved) = serde_json::from_str::<Value>(&text) else { return false };
    run(app, &saved).await
}

/// "Restore again" (AC-15 failure): the last checkpoint once more, sessions that exist kept.
pub async fn again(app: &Shared) -> Result<(), String> {
    let text = std::fs::read_to_string(path(app)).map_err(|e| format!("No layout to restore: {e}"))?;
    let saved: Value = serde_json::from_str(&text).map_err(|e| format!("layout.json cannot be read: {e}"))?;
    run(app, &saved).await;
    Ok(())
}

pub fn usable(dir: &str) -> bool {
    std::path::Path::new(dir).is_dir() && crate::tmux::args::check_text(dir).is_ok()
}

/// Recreates the sessions of `saved` on terminum's tmux; writes and keeps the report.
async fn run(app: &Shared, saved: &Value) -> bool {
    let Link::Local { tmux, socket, conf } = &app.tmux.local.link else { return false };
    let exec = Exec::Local { tmux, socket, conf: Some(conf) };
    let home = crate::config::home().display().to_string();
    let existing: Vec<String> = exec.run(&["list-sessions", "-F", "#{session_name}"]).await.unwrap_or_default().lines().map(String::from).collect();
    let mut m = Made::default();
    let kept = crate::restore_srv::kept(&exec).await;
    make(&exec, saved["sessions"].as_array().map(Vec::as_slice).unwrap_or_default(), &existing, None, usable, &home, &mut m).await;
    crate::page::projects::restarted(app, &app.tmux.local.key(""), &[m.panes.clone(), kept].concat()).await;
    let made = m.sessions;
    let mut text = format!("Restored {made} session{}, {} window{}.", if made == 1 { "" } else { "s" }, m.windows, if m.windows == 1 { "" } else { "s" });
    if !m.programs.is_empty() {
        let n = m.programs.len();
        text.push_str(&format!(" {n} pane{} ran {}: start {} again", if n == 1 { "" } else { "s" }, m.programs.join(", "), if n == 1 { "it" } else { "them" }));
    }
    *app.restore.report.lock() = None;
    add_report(app, &text, m);
    made > 0
}

/// Adds a restore's words to the report the pages show (and that a page connecting gets).
pub fn add_report(app: &Shared, text: &str, m: Made) {
    let mut r = app.restore.report.lock();
    let mut report = r.clone().unwrap_or_else(|| json!({"sessions": 0, "windows": 0, "programs": [], "errors": [], "notes": [], "text": ""}));
    report["at"] = json!(crate::auth::store::unix_now());
    report["sessions"] = json!(report["sessions"].as_u64().unwrap_or(0) + m.sessions as u64);
    report["windows"] = json!(report["windows"].as_u64().unwrap_or(0) + m.windows as u64);
    for (k, v) in [("programs", m.programs), ("errors", m.errors), ("notes", m.notes)] {
        if let Some(a) = report[k].as_array_mut() { a.extend(v.into_iter().map(Value::from)); }
    }
    let was = report["text"].as_str().unwrap_or("").to_string();
    report["text"] = json!(if was.is_empty() { text.to_string() } else { format!("{was} {text}") });
    let _ = crate::config::write_private(&app.cfg.dir.join("restore-report.json"), report.to_string().as_bytes());
    tracing::info!(event = "restore", text);
    *r = Some(report.clone());
    drop(r);
    app.pages.tell(&json!({"t": "restore", "report": report}).to_string());
}

/// A report this old is no news to a page that connects.
const REPORT_FOR: u64 = 600;

/// The report for a page that connects: a recent one that no page closed yet.
pub fn hello(app: &Shared, page: &crate::page::Page) {
    let fresh = |r: &Value| r["at"].as_u64().is_some_and(|at| crate::auth::store::unix_now().saturating_sub(at) < REPORT_FOR);
    if let Some(r) = app.restore.report.lock().clone().filter(fresh) {
        page.send_json(&json!({"t": "restore", "report": r}));
    }
}

/// A page closed the report: it is gone from every page and no page that connects gets it.
pub fn seen(app: &Shared) {
    if app.restore.report.lock().take().is_some() {
        let _ = std::fs::remove_file(app.cfg.dir.join("restore-report.json"));
        app.pages.tell(&json!({"t": "restore", "report": null}).to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_boot_has_an_id() {
        assert!(!boot_id().is_empty());
    }

    #[test]
    fn missing_or_odd_folders_are_not_used() {
        assert!(usable("/tmp"));
        assert!(!usable("/nonexistent-folder-of-terminum"));
    }
}
