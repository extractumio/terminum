// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The layout survives a reboot (AC-15). terminumd writes `state/layout.json` within 5 s of
//! a change: sessions, windows (name, `#{window_layout}`), panes (folder, the program's kind:
//! shell, an ssh recipe, other) and the boot it was seen in. When terminumd starts in a new
//! boot with no local server, it recreates the sessions with shells in their folders,
//! applies each window's layout and writes a report the first page shows. Never stored:
//! output, history, environment, command lines other than a checked ssh recipe.

use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::tmux::server::{plain, Link};

const EVERY: Duration = Duration::from_secs(5);
/// with a quick shell's mark (AC-88): those are not restored
const WINDOWS: &str = "#{session_name}\t#{window_id}\t#{@tm-quick}\t#{window_name}\t#{window_layout}";
const PANES: &str = "#{session_name}\t#{window_id}\t#{pane_id}\t#{pane_current_command}\t#{pane_tty}\t#{pane_current_path}";

#[derive(Default)]
pub struct Restore {
    /// the last checkpoint written (its JSON), to write only changes
    last: Mutex<String>,
    /// the report of the last restore, until a page has shown it
    pub report: Mutex<Option<Value>>,
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
async fn checkpoint(app: &Shared) -> Option<Value> {
    let mut answers = app.tmux.local.bulk()?.run_all(vec![format!("list-windows -a -F '{WINDOWS}'"), format!("list-panes -a -F '{PANES}'")]).await.into_iter();
    let (wins, panes) = (answers.next()?.ok()?, answers.next()?.ok()?);
    let fields: Vec<Vec<&str>> = panes.iter().map(|l| l.splitn(6, '\t').collect::<Vec<_>>()).filter(|g| g.len() == 6).collect();
    let ssh_ttys: Vec<&str> = fields.iter().filter(|g| g[3] == "ssh").map(|g| g[4]).collect();
    let procs = ssh_commands(&ssh_ttys).await;
    let mut sessions: Vec<Value> = Vec::new();
    for w in &wins {
        let f: Vec<&str> = w.splitn(5, '\t').collect();
        if f.len() < 5 || !f[2].is_empty() {
            continue;
        }
        let f = [f[0], f[1], f[3], f[4]];
        let mut ps = Vec::new();
        for p in fields.iter().filter(|g| g[1] == f[1]) {
            let mut k = kind(p[3], p[4], &procs);
            k["dir"] = json!(p[5]);
            ps.push(k);
        }
        let win = json!({"name": f[2], "layout": f[3], "panes": ps});
        match sessions.iter_mut().find(|s| s["name"] == f[0]) {
            Some(s) => s["windows"].as_array_mut().expect("array").push(win),
            None => sessions.push(json!({"name": f[0], "windows": [win]})),
        }
    }
    let hosts: Vec<String> = app.tmux.hosts().iter().map(|h| h.name.clone()).collect();
    Some(json!({"version": 1, "boot": boot_id(), "sessions": sessions, "hosts": hosts}))
}

/// Writes the checkpoint within 5 s of a change, while restore is on.
pub async fn follow(app: Shared) {
    loop {
        tokio::time::sleep(EVERY).await;
        if !enabled(&app) || app.tmux_problem.lock().is_some() {
            continue;
        }
        let Some(mut v) = checkpoint(&app).await else { continue };
        let text = v.to_string();
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

/// At start with no local server: restores the last checkpoint when it is of another boot
/// (a reboot, not a restart of terminumd or a kill-server). True when sessions were made.
pub async fn at_start(app: &Shared) -> bool {
    if !enabled(app) || app.restore.report.lock().is_some() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(path(app)) else { return false };
    let Ok(saved) = serde_json::from_str::<Value>(&text) else { return false };
    if saved["boot"].as_str().unwrap_or("") == boot_id() {
        return false;
    }
    run(app, &saved).await
}

/// "Restore again" (AC-15 failure): the last checkpoint once more, sessions that exist kept.
pub async fn again(app: &Shared) -> Result<(), String> {
    let text = std::fs::read_to_string(path(app)).map_err(|e| format!("No layout to restore: {e}"))?;
    let saved: Value = serde_json::from_str(&text).map_err(|e| format!("layout.json cannot be read: {e}"))?;
    run(app, &saved).await;
    Ok(())
}

async fn tmux(app: &Shared, args: &[&str]) -> Result<String, String> {
    let Link::Local { tmux, socket, conf } = &app.tmux.local.link else { return Err("no local server".into()) };
    let mut c = plain(tmux);
    c.args(["-L", socket, "-f"]).arg(conf).args(args);
    let out = c.output().await.map_err(|e| e.to_string())?;
    if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}

fn usable(dir: &str) -> bool {
    std::path::Path::new(dir).is_dir() && crate::tmux::args::check_text(dir).is_ok()
}

/// Recreates the sessions of `saved`; writes and keeps the report.
async fn run(app: &Shared, saved: &Value) -> bool {
    let home = crate::config::home().display().to_string();
    let (mut made, mut windows, mut programs, mut errors, mut notes) = (0, 0, Vec::new(), Vec::new(), Vec::new());
    let existing = tmux(app, &["list-sessions", "-F", "#{session_name}"]).await.unwrap_or_default();
    for s in saved["sessions"].as_array().into_iter().flatten() {
        let name = s["name"].as_str().unwrap_or("");
        if name.is_empty() || existing.lines().any(|l| l == name) {
            continue;
        }
        let mut first = true;
        for w in s["windows"].as_array().into_iter().flatten() {
            let panes = w["panes"].as_array().cloned().unwrap_or_default();
            let dir_of = |p: &Value| {
                let d = p["dir"].as_str().unwrap_or("");
                if usable(d) { d.to_string() } else { home.clone() }
            };
            for p in &panes {
                if !usable(p["dir"].as_str().unwrap_or("")) {
                    notes.push(format!("{} is gone: opened in ~", p["dir"].as_str().unwrap_or("?")));
                }
                match p["kind"].as_str() {
                    Some("ssh") => programs.push(format!("ssh {}", p["recipe"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default())),
                    Some("other") => programs.push(p["program"].as_str().unwrap_or("a program").to_string()),
                    _ => {}
                }
            }
            let d0 = panes.first().map_or(home.clone(), dir_of);
            let wname = w["name"].as_str().unwrap_or("");
            let target = format!("={name}:");
            let made_win = if first {
                tmux(app, &["new-session", "-d", "-s", name, "-n", wname, "-c", &d0, "-P", "-F", "#{window_id}"]).await
            } else {
                tmux(app, &["new-window", "-d", "-t", &target, "-n", wname, "-c", &d0, "-P", "-F", "#{window_id}"]).await
            };
            let wid = match made_win {
                Ok(id) => id,
                Err(e) => {
                    errors.push(format!("{name}: {e}"));
                    if first { break } else { continue }
                }
            };
            if first {
                made += 1;
            }
            first = false;
            windows += 1;
            for p in panes.iter().skip(1) {
                if let Err(e) = tmux(app, &["split-window", "-d", "-t", &wid, "-c", &dir_of(p)]).await {
                    errors.push(format!("{name}: {e}"));
                }
            }
            if let Some(layout) = w["layout"].as_str().filter(|l| !l.is_empty()) {
                let _ = tmux(app, &["select-layout", "-t", &wid, layout]).await;
            }
        }
    }
    let mut text = format!("Restored {made} session{}, {windows} window{}.", if made == 1 { "" } else { "s" }, if windows == 1 { "" } else { "s" });
    if !programs.is_empty() {
        text.push_str(&format!(" {} pane{} ran {}: start {} again", programs.len(), if programs.len() == 1 { "" } else { "s" }, programs.join(", "), if programs.len() == 1 { "it" } else { "them" }));
    }
    let report = json!({"at": crate::auth::store::unix_now(), "sessions": made, "windows": windows, "programs": programs, "errors": errors, "notes": notes, "text": text});
    let _ = crate::config::write_private(&app.cfg.dir.join("restore-report.json"), report.to_string().as_bytes());
    tracing::info!(event = "restore", sessions = made, windows, errors = errors.len());
    *app.restore.report.lock() = Some(report.clone());
    app.pages.tell(&json!({"t": "restore", "report": report}).to_string());
    made > 0
}

/// The report for a page that connects (the first one shows it).
pub fn hello(app: &Shared, page: &crate::page::Page) {
    if let Some(r) = app.restore.report.lock().clone() {
        page.send_json(&json!({"t": "restore", "report": r}));
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
