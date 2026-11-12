// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Each agent kind's usage command (`terminum-dispatcher.md` AC-168): a line the user may change in settings.json
//! (`dispatcher.usageCommands`), run on this Mac to read the kind's subscription windows without a model call:
//! when terminumd starts or the line changes, every RUNNING_EVERY while a run of that kind has an open task, as soon as one of its tasks
//! ends, else every IDLE_EVERY; only for kinds the pool has. Its numbers alone are kept, as a session's are.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

use super::subscriptions::Window;
use super::{runs, store, usageread};
use crate::app::Shared;
use crate::tasks::ops;

/// How often a kind is asked while a run of it works on a task, and otherwise.
const RUNNING_EVERY: i64 = 600;
const IDLE_EVERY: i64 = 3600;
/// The least time between two asks of a kind: tasks ending one after another ask once.
const LEAST_GAP: i64 = 60;
/// The longest a usage command may take, and the most of its output read.
const MOST_TIME: Duration = Duration::from_secs(60);
const MOST_OUT: u64 = 1 << 20;
/// The file in the state folder the last readings are kept in.
const FILE: &str = "subscriptions.json";

/// What is known of asking each kind; a restart asks anew.
#[derive(Default)]
pub struct Asking {
    kinds: Mutex<BTreeMap<String, Kind>>,
    loaded: AtomicBool,
}

#[derive(Default)]
struct Kind {
    /// when it was last asked
    asked: Option<i64>,
    /// its command runs now
    busy: bool,
    /// its live runs' open tasks at the last beat, by run
    open: HashMap<String, usize>,
    /// the command it was last asked by: a changed one asks at once
    line: String,
}

/// Whether a kind last asked at `asked` is due at `now`: with a run of it working, or one of its tasks just ended.
fn due(asked: Option<i64>, now: i64, working: bool, ended: bool) -> bool {
    let Some(asked) = asked else { return true };
    let gap = now - asked;
    gap >= LEAST_GAP && (ended || gap >= if working { RUNNING_EVERY } else { IDLE_EVERY })
}

/// Whether a task ended since the last beat: a run has fewer open tasks, or is gone.
fn ended(was: &HashMap<String, usize>, now: &HashMap<String, usize>) -> bool {
    was.iter().any(|(run, n)| now.get(run).is_none_or(|m| m < n))
}

/// Each walk's beat: asks every kind that is due, each in a task of its own; the walk does not wait for them.
pub async fn beat(app: &Shared) {
    if !app.dispatcher.asking.loaded.swap(true, Ordering::Relaxed) {
        let path = crate::config::app_dir().join(FILE);
        if let Ok(Ok(file)) = tokio::fs::read(&path).await.map(|b| serde_json::from_slice::<Value>(&b)) {
            app.dispatcher.subscriptions.load(&file);
        }
    }
    let Ok((kinds, live)) = store(app, |tx| Ok((ops::rows(tx, "SELECT DISTINCT kind FROM agents", [], |r| r.get::<_, String>(0))?, runs::live(tx)?))).await else { return };
    let lines = app.looks.settings.read().usage_commands.clone();
    // a test's private terminumd (its own TM_APP_DIR) runs only what its settings.json names, never the real agents
    let private = std::env::var_os("TM_APP_DIR").is_some();
    let now = ops::now();
    for kind in kinds {
        let open: HashMap<String, usize> = live.iter().filter(|r| r.kind == kind).map(|r| (r.id.clone(), r.open.len())).collect();
        let mut all = app.dispatcher.asking.kinds.lock();
        let k = all.entry(kind.clone()).or_default();
        let (was, working) = (std::mem::replace(&mut k.open, open), k.open.values().any(|n| *n > 0));
        let by_default = [crate::settings::USAGE_CLAUDE, crate::settings::USAGE_CODEX];
        let Some(line) = lines.get(&kind).filter(|l| !l.is_empty() && !(private && by_default.contains(&l.as_str()))).cloned() else { continue };
        let asked = k.asked.filter(|_| k.line == line);
        if k.busy || !due(asked, now, working, ended(&was, &k.open)) {
            continue;
        }
        (k.busy, k.asked, k.line) = (true, Some(now), line.clone());
        let app = app.clone();
        tokio::spawn(async move {
            let got = ask(&kind, &line, &crate::config::app_dir().join("usage"), MOST_TIME).await;
            let at = ops::now();
            match got {
                Ok(windows) => {
                    if app.dispatcher.subscriptions.heard(&kind, windows, at, "usage") {
                        told(&app).await;
                    }
                }
                Err(why) => {
                    tracing::warn!(event = "dispatcher.usage", kind = %kind, error = %why);
                    app.dispatcher.subscriptions.failed(&kind, why, at);
                    told(&app).await;
                }
            }
            if let Some(k) = app.dispatcher.asking.kinds.lock().get_mut(&kind) {
                k.busy = false;
            }
        });
    }
}

/// The readings changed: pages are told, and the state file keeps them.
pub async fn told(app: &Shared) {
    let s = &app.dispatcher.subscriptions;
    app.pages.tell(&json!({"t": "subscriptions", "subscriptions": s.shown()}).to_string());
    let (path, kept) = (crate::config::app_dir().join(FILE), s.kept().to_string());
    if let Err(e) = tokio::task::spawn_blocking(move || crate::config::write_private(&path, kept.as_bytes())).await.unwrap_or_else(|e| Err(std::io::Error::other(e))) {
        tracing::warn!(event = "dispatcher.usage.save", error = %e);
    }
}

/// The PATH a usage command gets: terminumd's, and the folders the agents' installers use (launchd's is short).
fn path() -> String {
    let home = crate::config::home();
    let mine = std::env::var("PATH").unwrap_or_default();
    let extra = [home.join(".local/bin"), home.join(".npm-global/bin"), home.join("bin")].map(|p| p.display().to_string());
    let mut dirs: Vec<String> = mine.split(':').filter(|d| !d.is_empty()).map(str::to_string).collect();
    for d in extra.into_iter().chain(["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"].map(str::to_string)) {
        if !dirs.contains(&d) {
            dirs.push(d);
        }
    }
    dirs.join(":")
}

/// Runs `kind`'s usage command `line` through `sh -c`, in folder `dir` (one of its own), outside tmux, its process
/// group ended after `most` at the latest; its windows, or why there are none.
pub async fn ask(kind: &str, line: &str, dir: &std::path::Path, most: Duration) -> Result<Vec<Window>, String> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let codex = kind == "codex";
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(line).current_dir(dir).env("PATH", path()).env_remove("TMUX").env_remove("TMUX_PANE")
        .stdin(if codex { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped())
        .process_group(0).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start /bin/sh: {e}"))?;
    let group = child.id().map(|p| p as libc::pid_t);
    let mut stderr = child.stderr.take().map(|e| e.take(4096));
    let got = tokio::time::timeout(most, async {
        let out = child.stdout.take().expect("piped").take(MOST_OUT);
        if codex {
            // its answer comes while its input is open: the question, then read up to the answer, then EOF
            let mut input = child.stdin.take().expect("piped");
            input.write_all(usageread::CODEX_ASK.as_bytes()).await.map_err(|e| format!("its input closed: {e}"))?;
            let mut lines = BufReader::new(out).lines();
            while let Ok(Some(l)) = lines.next_line().await {
                if let Ok(v) = serde_json::from_str::<Value>(&l) {
                    if v["id"] == 2 {
                        drop(input);
                        return usageread::of_codex(&v);
                    }
                }
            }
            Err(said(&mut child, &mut stderr, "it ended without an answer").await)
        } else {
            let mut text = String::new();
            BufReader::new(out).read_to_string(&mut text).await.map_err(|e| format!("its output: {e}"))?;
            usageread::of_claude(&text)
        }
    }).await;
    let result = match got {
        Ok(Ok(w)) => Ok(w),
        Ok(Err(why)) if !codex => Err(said(&mut child, &mut stderr, &why).await),
        Ok(Err(why)) => Err(why),
        Err(_) => Err(format!("no answer in {} s", most.as_secs_f64())),
    };
    if let Some(g) = group {
        // its own group, made above: what it started ends with it
        // SAFETY: killpg has no preconditions; `g` is the group this child leads
        unsafe { libc::killpg(g, libc::SIGTERM) };
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
    result
}

/// `why`, or what the command said on its way out when it failed: its exit and its error output's first line.
async fn said(child: &mut tokio::process::Child, stderr: &mut Option<tokio::io::Take<tokio::process::ChildStderr>>, why: &str) -> String {
    let status = tokio::time::timeout(Duration::from_secs(5), child.wait()).await.ok().and_then(Result::ok);
    let mut err = String::new();
    if let Some(e) = stderr.as_mut() {
        let _ = tokio::time::timeout(Duration::from_secs(1), e.read_to_string(&mut err)).await;
    }
    let first = err.lines().find(|l| !l.trim().is_empty()).map(|l| l.chars().take(160).collect::<String>());
    match (status.and_then(|s| s.code()), first) {
        (Some(127), _) => "its program was not found on terminumd's PATH: set dispatcher.usageCommands with its full path".into(),
        (Some(code), Some(line)) if code != 0 => format!("it ended with exit {code}: {line}"),
        (Some(code), None) if code != 0 => format!("it ended with exit {code}"),
        _ => why.to_string(),
    }
}

#[cfg(test)]
#[path = "usageask_tests.rs"]
mod tests;
