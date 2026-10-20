// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The layout of every server terminum shows, kept and made again (AC-15): besides terminum's
//! own (restore.rs), a host's `terminum` session and the sessions of the user's own tmux
//! attached by name (AC-89), on this Mac and on hosts. Each server's sessions (windows: name,
//! layout; panes: folder, kind) are written into layout.json's `servers` under the server's id
//! while it is read; a server that cannot be read keeps what was written before. They are made
//! again only where the whole tmux is gone: a server of the user's own that does not run, a
//! host whose tmux server started after its checkpoint (a reboot, a killed server). A session
//! or window the user closed while its server runs stays closed, and so does the last pane
//! whose closing ended its tmux (tmux's own `%exit` while one pane was shown).

use std::path::Path;

use serde_json::{json, Value};

use crate::app::Shared;
use crate::tmux::args;
use crate::tmux::server::{plain, Link, Server, HOST_SESSION};

pub const WINDOWS: &str = "#{session_name}\t#{window_id}\t#{@tm-quick}\t#{window_name}\t#{window_layout}";
pub const PANES: &str = "#{session_name}\t#{window_id}\t#{pane_id}\t#{pane_current_command}\t#{pane_tty}\t#{pane_current_path}";

/// How tmux commands reach a server: its own process (`tmux -L <socket>`, a config file for
/// terminum's), or a control client of it (a host's).
pub enum Exec<'a> {
    Local { tmux: &'a Path, socket: &'a str, conf: Option<&'a Path> },
    Ctl(&'a Server),
}

impl Exec<'_> {
    pub async fn run(&self, words: &[&str]) -> Result<String, String> {
        match self {
            Exec::Local { tmux, socket, conf } => {
                let mut c = plain(tmux);
                c.args(["-L", socket]);
                if let Some(f) = conf {
                    c.arg("-f").arg(f);
                }
                let out = c.args(words).output().await.map_err(|e| e.to_string())?;
                if out.status.success() {
                    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
                } else {
                    Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
                }
            }
            Exec::Ctl(s) => s.run(words.iter().map(|w| args::word(w)).collect::<Vec<_>>().join(" ")).await.map(|l| l.join("\n")),
        }
    }
}

/// The panes running on `exec`'s server before a restore makes any, each paired with itself:
/// put after the panes made again, they keep their projects and bookmarks (a restore again
/// passes over the sessions still running).
pub async fn kept(exec: &Exec<'_>) -> Vec<(String, String)> {
    let listed = exec.run(&["list-panes", "-a", "-F", "#{pane_id}"]).await.unwrap_or_default();
    listed.lines().map(str::trim).filter(|p| p.starts_with('%')).map(|p| (p.to_string(), p.to_string())).collect()
}

/// The kind of a pane's program for the checkpoint (a shell, else another program: never its
/// command line). terminum's own server says more (restore.rs: ssh recipes).
pub fn simple_kind(command: &str) -> Value {
    if crate::tmux::titles::SHELLS.contains(&command.trim()) { json!({"kind": "shell"}) } else { json!({"kind": "other", "program": command.trim()}) }
}

/// The sessions of `wins` and `panes` (the lists of WINDOWS and PANES) that `keep` takes, in
/// the checkpoint's shape; `kind(command, tty)` describes a pane's program.
pub fn sessions_of(wins: &[String], panes: &[String], keep: impl Fn(&str) -> bool, kind: impl Fn(&str, &str) -> Value) -> Vec<Value> {
    let fields: Vec<Vec<&str>> = panes.iter().map(|l| l.splitn(6, '\t').collect::<Vec<_>>()).filter(|g| g.len() == 6).collect();
    let mut sessions: Vec<Value> = Vec::new();
    for w in wins {
        let f: Vec<&str> = w.splitn(5, '\t').collect();
        if f.len() < 5 || !f[2].is_empty() || !keep(f[0]) {
            continue;                                   // a quick shell (AC-88) is not kept
        }
        let ps: Vec<Value> = fields.iter().filter(|g| g[1] == f[1]).map(|p| {
            let mut k = kind(p[3], p[4]);
            k["dir"] = json!(p[5]);
            k["id"] = json!(p[2]);                      // its pane's id, for what keeps pane keys (projects, AC-92)
            k
        }).collect();
        let win = json!({"name": f[3], "layout": f[4], "panes": ps});
        match sessions.iter_mut().find(|s| s["name"] == f[0]) {
            Some(s) => s["windows"].as_array_mut().expect("array").push(win),
            None => sessions.push(json!({"name": f[0], "windows": [win]})),
        }
    }
    sessions
}

/// What one making-again did, for the report.
#[derive(Default)]
pub struct Made {
    pub sessions: usize,
    pub windows: usize,
    pub programs: Vec<String>,
    pub errors: Vec<String>,
    pub notes: Vec<String>,
    /// the panes made again: (the checkpoint's pane id, the new one)
    pub panes: Vec<(String, String)>,
}

/// Makes `sessions` on the server `exec` reaches, but those in `existing`; `into` names a
/// session that exists already and takes the windows instead (a host's fresh `terminum`).
/// `dir_ok(dir)`: whether a folder can be used (else the home folder, with a note).
pub async fn make(exec: &Exec<'_>, sessions: &[Value], existing: &[String], into: Option<&str>, dir_ok: impl Fn(&str) -> bool, home: &str, out: &mut Made) {
    for s in sessions {
        let name = s["name"].as_str().unwrap_or("");
        let fill = into == Some(name);
        if name.is_empty() || args::check_session_name(name).is_err() || (existing.iter().any(|e| e == name) && !fill) {
            continue;
        }
        let mut first = !fill;
        for w in s["windows"].as_array().into_iter().flatten() {
            let panes = w["panes"].as_array().cloned().unwrap_or_default();
            let dir_of = |p: &Value| {
                let d = p["dir"].as_str().unwrap_or("");
                if !d.is_empty() && dir_ok(d) { d.to_string() } else { home.to_string() }
            };
            for p in &panes {
                let d = p["dir"].as_str().unwrap_or("");
                if !d.is_empty() && !dir_ok(d) {
                    out.notes.push(format!("{d} is gone: opened in ~"));
                }
                match p["kind"].as_str() {
                    Some("ssh") => out.programs.push(format!("ssh {}", p["recipe"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default())),
                    Some("other") => out.programs.push(p["program"].as_str().unwrap_or("a program").to_string()),
                    _ => {}
                }
            }
            let d0 = panes.first().map_or(home.to_string(), dir_of);
            let wname = w["name"].as_str().unwrap_or("");
            let target = format!("={name}:");
            let made = if first {
                exec.run(&["new-session", "-d", "-s", name, "-n", wname, "-c", &d0, "-P", "-F", "#{window_id} #{pane_id}"]).await
            } else {
                exec.run(&["new-window", "-d", "-t", &target, "-n", wname, "-c", &d0, "-P", "-F", "#{window_id} #{pane_id}"]).await
            };
            let old = |p: Option<&Value>| p.and_then(|p| p["id"].as_str()).map(str::to_string);
            let wid = match made {
                Ok(ids) => {
                    let (wid, pane) = ids.split_once(' ').unwrap_or((ids.as_str(), ""));
                    if let Some(o) = old(panes.first()) {
                        out.panes.push((o, pane.to_string()));
                    }
                    wid.to_string()
                }
                Err(e) => {
                    out.errors.push(format!("{name}: {e}"));
                    if first { break } else { continue }
                }
            };
            if first {
                out.sessions += 1;
            }
            first = false;
            out.windows += 1;
            for p in panes.iter().skip(1) {
                match exec.run(&["split-window", "-d", "-t", &wid, "-c", &dir_of(p), "-P", "-F", "#{pane_id}"]).await {
                    Ok(new) => out.panes.extend(old(Some(p)).map(|o| (o, new))),
                    Err(e) => out.errors.push(format!("{name}: {e}")),
                }
            }
            if let Some(layout) = w["layout"].as_str().filter(|l| !l.is_empty()) {
                let _ = exec.run(&["select-layout", "-t", &wid, layout]).await;
            }
        }
    }
}

/// The checkpoint of every other server that can be read now (a host's `terminum`, the
/// sessions of the user's own tmux attached by name), over the ones `prev` kept.
pub async fn checkpoint(app: &Shared, prev: &Value) -> Value {
    let mut servers = prev.as_object().cloned().unwrap_or_default();
    for s in app.tmux.all().into_iter().filter(|s| !std::ptr::eq(s.as_ref(), app.tmux.local.as_ref())) {
        let Some(bulk) = s.bulk() else { continue };
        let mut answers = bulk.run_all(vec![format!("list-windows -a -F '{WINDOWS}'"), format!("list-panes -a -F '{PANES}'")]).await.into_iter();
        let (Some(Ok(wins)), Some(Ok(panes))) = (answers.next(), answers.next()) else { continue };
        // a host's tmux started anew is kept as it was until its layout was made again (host())
        if !s.is_local() && let Some(started) = started(&s).await {
            let at = servers.get(&s.id()).and_then(|v| v["at"].as_i64()).unwrap_or(i64::MAX);
            if started > at && !app.restore.was_tried(&format!("{}@{started}", s.id())) {
                continue;
            }
        }
        let sessions = sessions_of(&wins, &panes, |name| s.shows(name), |c, _| simple_kind(c));
        if sessions.is_empty() {
            continue;                                   // nothing of it shown now: what was kept stays
        }
        servers.insert(s.id(), json!({"host": (!s.is_local()).then(|| s.name.clone()), "at": crate::auth::store::unix_now(), "sessions": sessions}));
    }
    Value::Object(servers)
}

/// When the server started (Unix seconds): a session closed and made again by terminumd while it
/// runs (the user closed its last window) is no new server.
async fn started(server: &Server) -> Option<i64> {
    let lines = server.run("list-sessions -F '#{start_time}'").await.ok()?;
    lines.first()?.trim().parse().ok()
}

fn saved(app: &Shared, id: &str) -> Option<Value> {
    let text = std::fs::read_to_string(app.cfg.dir.join("layout.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    v["servers"].get(id).cloned()
}

fn report(app: &Shared, place: &str, m: Made) {
    if m.sessions + m.windows == 0 && m.errors.is_empty() {
        return;
    }
    let text = format!("Restored {place}: {} session{}, {} window{}.", m.sessions, if m.sessions == 1 { "" } else { "s" },
        m.windows, if m.windows == 1 { "" } else { "s" });
    tracing::info!(event = "restore", place, sessions = m.sessions, windows = m.windows, errors = m.errors.len());
    crate::restore::add_report(app, &text, m);
}

/// The user's own tmux on this Mac does not run (a reboot, its server killed): its attached
/// sessions are made again from their checkpoint, once while terminumd runs.
pub async fn users_local(app: &Shared, server: &Server) {
    let by_user = server.took_closed_by_user();
    let Link::Local { tmux, socket, .. } = &server.link else { return };
    if !app.looks.settings.read().restore {
        return;
    }
    let Some(saved) = saved(app, &server.id()) else { return };
    if !app.restore.first_try(&format!("{}@{}", server.id(), saved["at"])) || by_user {
        return;                                         // made again already, or the user closed its last pane
    }
    let only = server.only.lock().clone().unwrap_or_default();
    let wanted: Vec<Value> = saved["sessions"].as_array().into_iter().flatten().filter(|s| only.iter().any(|o| s["name"] == o.as_str())).cloned().collect();
    if wanted.is_empty() {
        return;
    }
    let mut m = Made::default();
    let home = crate::config::home().display().to_string();
    let exec = Exec::Local { tmux, socket, conf: None };
    let kept = kept(&exec).await;
    make(&exec, &wanted, &[], None, crate::restore::usable, &home, &mut m).await;
    crate::page::projects::restarted(app, &server.key(""), &[m.panes.clone(), kept].concat()).await;
    report(app, &format!("your tmux{}", if socket == "default" { String::new() } else { format!(" -L {socket}") }), m);
}

/// A host attached again: when its tmux server started after its checkpoint (a reboot, a killed
/// server), the `terminum` session's windows and the user's attached sessions there are made
/// again. `seen`: the sessions running there.
pub async fn host(app: &Shared, server: &Server, seen: &[(String, String)]) {
    let by_user = server.took_closed_by_user();
    if server.is_local() || !app.looks.settings.read().restore {
        return;
    }
    let Some(saved) = saved(app, &server.id()) else { return };
    let at = saved["at"].as_i64().unwrap_or(i64::MAX);
    let started = started(server).await.unwrap_or(0);
    // the user closed its last pane and its tmux ended with it: nothing to make again
    if started <= at || !app.restore.first_try(&format!("{}@{started}", server.id())) || by_user {
        return;
    }
    let existing: Vec<String> = seen.iter().map(|(n, _)| n.clone()).collect();
    let fresh = server.run(format!("list-windows -t ={HOST_SESSION} -F '#{{window_id}}'")).await.unwrap_or_default();
    let sessions: Vec<Value> = saved["sessions"].as_array().cloned().unwrap_or_default();
    let mut m = Made::default();
    let home = server.home().await.unwrap_or_default();
    let kept = kept(&Exec::Ctl(server)).await;
    make(&Exec::Ctl(server), &sessions, &existing, Some(HOST_SESSION), |_| true, if home.is_empty() { "~" } else { &home }, &mut m).await;
    crate::page::projects::restarted(app, &server.key(""), &[m.panes.clone(), kept].concat()).await;
    // the fresh session's own first window goes once the kept ones are there
    if m.windows > 0 && fresh.len() == 1 {
        let _ = server.run(format!("kill-window -t {}", args::word(&fresh[0]))).await;
    }
    report(app, &format!("host {}", server.name), m);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_kept_with_their_windows_and_folders_but_quick_shells() {
        let wins = ["main\t@1\t\tapi\tlayout1", "main\t@2\t%9\tquick\tlayout2", "work\t@3\t\tlogs\tlayout3", "other\t@4\t\tx\tl"].map(String::from);
        let panes = ["main\t@1\t%1\tzsh\t/dev/ttys1\t/Users/alex/api", "main\t@1\t%2\tnode\t/dev/ttys2\t/Users/alex/api/web",
                     "main\t@2\t%9\tzsh\t/dev/ttys3\t/tmp", "work\t@3\t%4\tzsh\t/dev/ttys4\t/var/log"].map(String::from);
        let s = sessions_of(&wins, &panes, |n| n != "other", |c, _| simple_kind(c));
        assert_eq!(s.len(), 2, "a session the server does not show is not kept");
        assert_eq!(s[0]["windows"].as_array().unwrap().len(), 1, "the quick shell's window is not kept");
        assert_eq!(s[0]["windows"][0]["panes"][1], json!({"kind": "other", "program": "node", "dir": "/Users/alex/api/web", "id": "%2"}));
        assert_eq!(s[1]["windows"][0]["panes"][0], json!({"kind": "shell", "dir": "/var/log", "id": "%4"}));
    }
}
