// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The layout survives a reboot and a killed tmux (AC-15). terminumd writes `state/layout.json`
//! within 5 s of a change: sessions, windows (name, `#{window_layout}`), panes (folder, the
//! program's kind: shell, an ssh recipe, other), the boot it was seen in, and the other servers
//! it shows (restore_srv.rs: hosts, the user's own tmux). When terminum's tmux has no session
//! (after a reboot, or its server was killed), terminumd recreates the sessions with shells in
//! their folders, applies each window's layout and writes a report the first page shows. Never
//! stored: output, history, environment, command lines other than a checked ssh recipe (the
//! last lines of panes are kept apart, lastlines.rs: the checkpoint names the server's identity).

use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::tmux::server::Link;

const EVERY: Duration = Duration::from_secs(5);
use crate::restore_srv::{make, sessions_of, Exec, Made, IDENTITY, PANES, WINDOWS};

/// How the restore of one server's start went (its key: `<server id>@<start or checkpoint>`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Attempt {
    Running,
    /// made again, or nothing was missing
    Done,
    /// an error on the connection `generation` (Server::generation); `made`: some of it is back
    Failed { generation: u64, made: bool },
}

#[derive(Default)]
pub struct Restore {
    /// the last checkpoint written (its JSON), to write only changes
    last: Mutex<String>,
    /// the report of the last restore, until a page has shown it
    pub report: Mutex<Option<Value>>,
    /// each server's start made again while terminumd runs, and how it went
    attempts: Mutex<std::collections::HashMap<String, Attempt>>,
    /// one restore at a time: a reboot's, a host's, the user's own tmux, a Restore again
    one: tokio::sync::Mutex<()>,
}

impl Restore {
    /// Whether `what` is to be made again by itself on the connection `generation`: never
    /// tried, or failed with nothing back on another connection (a link that dropped meanwhile).
    pub fn due(&self, what: &str, generation: u64) -> bool {
        match self.attempts.lock().get(what) {
            None => true,
            Some(Attempt::Failed { generation: g, made: false }) => *g != generation,
            Some(_) => false,
        }
    }

    pub fn begin(&self, what: &str) {
        self.attempts.lock().insert(what.to_string(), Attempt::Running);
    }

    /// The outcome of `what`'s attempt `m` on the connection `generation`.
    pub fn end(&self, what: &str, m: &Made, generation: u64) {
        let a = if m.errors.is_empty() { Attempt::Done } else { Attempt::Failed { generation, made: m.sessions + m.windows > 0 } };
        self.attempts.lock().insert(what.to_string(), a);
    }

    /// The one restore's turn for `what`, when it is due by itself (checked before the wait and again
    /// after it: another restore may have made it meanwhile).
    pub async fn turn_if_due(&self, what: &str, generation: u64) -> Option<tokio::sync::MutexGuard<'_, ()>> {
        if !self.due(what, generation) {
            return None;
        }
        let one = self.one.lock().await;
        self.due(what, generation).then_some(one)
    }

    /// Whether `what` waits for its layout: being made, or failed with nothing back (its
    /// checkpoint is all there is of it, so it is not written over).
    pub fn waits(&self, what: &str) -> bool {
        matches!(self.attempts.lock().get(what), None | Some(Attempt::Running | Attempt::Failed { made: false, .. }))
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
    let mut answers = app.tmux.local.bulk()?.run_all(vec![format!("list-windows -a -F '{WINDOWS}'"), format!("list-panes -a -F '{PANES}'"), IDENTITY.into()]).await.into_iter();
    let (wins, panes) = (answers.next()?.ok()?, answers.next()?.ok()?);
    let identity = answers.next().and_then(Result::ok).and_then(|l| l.into_iter().next()).unwrap_or_default();
    let ssh_ttys: Vec<&str> = panes.iter().map(|l| l.splitn(7, '\t').collect::<Vec<_>>()).filter(|g| g.len() == 7 && g[3] == "ssh").map(|g| g[4]).collect();
    let procs = ssh_commands(&ssh_ttys).await;
    let sessions = sessions_of(&wins, &panes, |_| true, |c, tty| kind(c, tty, &procs));
    let hosts: Vec<String> = app.tmux.hosts().iter().map(|h| h.name.clone()).collect();
    let servers = crate::restore_srv::checkpoint(app, &prev["servers"]).await;
    Some(json!({"version": 2, "boot": boot_id(), "identity": identity.trim(), "sessions": sessions, "hosts": hosts, "servers": servers}))
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
        v["at"] = json!(crate::auth::store::unix_now());
        match crate::config::write_private(&path(&app), v.to_string().as_bytes()) {
            Ok(()) => *app.restore.last.lock() = text,
            Err(e) => tracing::warn!(event = "restore.checkpoint_failed", error = %e),
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
    let _one = app.restore.one.lock().await;
    run(app, &saved).await
}

/// "Restore again" (AC-15 failure): the last checkpoint once more on every server it names
/// that answers (terminum's, hosts, the user's own tmux), sessions that exist kept.
pub async fn again(app: &Shared) -> Result<(), String> {
    let text = std::fs::read_to_string(path(app)).map_err(|e| format!("No layout to restore: {e}"))?;
    let saved: Value = serde_json::from_str(&text).map_err(|e| format!("layout.json cannot be read: {e}"))?;
    let _one = app.restore.one.lock().await;
    run(app, &saved).await;
    for s in app.tmux.all().into_iter().filter(|s| !std::ptr::eq(s.as_ref(), app.tmux.local.as_ref())) {
        crate::restore_srv::again(app, &s, &saved["servers"]).await;
    }
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
    let sessions = saved["sessions"].as_array().map(Vec::as_slice).unwrap_or_default();
    let print = crate::restore_lines::commands(app, &exec, &app.tmux.local.id(), saved["identity"].as_str(), sessions).await;
    make(&exec, sessions, &existing, None, usable, &home, &print, &mut m).await;
    crate::restore_srv::hand_on(app, &exec, &app.tmux.local, saved, kept, &mut m).await;
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
    let mut text = text.to_string();
    let lines = format!("Last lines before the restart: a session's menu, Show Last {} Lines.", crate::lastlines::wanted(app));
    if m.lines > 0 && !was.contains(&lines) {
        text = format!("{text}{} {lines}", if text.ends_with('.') { "" } else { "." });
    }
    report["text"] = json!(if was.is_empty() { text.clone() } else { format!("{was} {text}") });
    if let Err(e) = crate::config::write_private(&app.cfg.dir.join("restore-report.json"), report.to_string().as_bytes()) {
        tracing::warn!(event = "restore.report_failed", error = %e);
        if let Some(a) = report["errors"].as_array_mut() { a.push(json!(format!("The report could not be kept in the state folder: {e}"))); }
    }
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
        if let Err(e) = crate::config::remove_if_there(&app.cfg.dir.join("restore-report.json")) {
            tracing::warn!(event = "restore.report_remove_failed", error = %e);
        }
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

    /// AC-15 failure: a server's restore that failed with nothing back is made again by
    /// itself once its link is another one; one that brought some back, or all, is not.
    #[test]
    fn ac15_a_failed_restore_is_retried_on_another_connection_only() {
        let r = Restore::default();
        let failed = |made| Made { sessions: made, errors: vec!["main: lost server".into()], ..Made::default() };
        assert!(r.due("devbox:default@100", 1) && r.waits("devbox:default@100"), "never tried");
        r.begin("devbox:default@100");
        assert!(!r.due("devbox:default@100", 1) && r.waits("devbox:default@100"), "being made");
        r.end("devbox:default@100", &failed(0), 1);
        assert!(!r.due("devbox:default@100", 1), "not again on the same link");
        assert!(r.due("devbox:default@100", 2), "again once the link is another one");
        assert!(r.waits("devbox:default@100"), "its checkpoint is kept for the retry");
        r.end("devbox:default@100", &failed(1), 2);
        assert!(!r.due("devbox:default@100", 3) && !r.waits("devbox:default@100"), "some of it back: Restore again only");
        r.end("devbox:default@100", &Made::default(), 3);
        assert_eq!(r.attempts.lock().get("devbox:default@100"), Some(&Attempt::Done));
        assert!(!r.due("devbox:default@100", 4) && !r.waits("devbox:default@100"));
    }

    #[test]
    fn missing_or_odd_folders_are_not_used() {
        assert!(usable("/tmp"));
        assert!(!usable("/nonexistent-folder-of-terminum"));
    }
}
