// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Each agent kind's usage command (`terminum-dispatcher.md` AC-168): a line the user may change in settings.json
//! (`dispatcher.usageCommands`), run on this Mac to read the kind's subscription windows without a model call:
//! when terminumd starts or the line changes, every RUNNING_EVERY while a run of that kind has an open task or its
//! last ask failed, as soon as one of its tasks ends, else every IDLE_EVERY; only for kinds the pool has. Its numbers
//! alone are kept, as a session's are, in the store's `meta` table.

use std::collections::{BTreeMap, HashMap};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

use super::subscriptions::{Readings, Via, Window};
use super::{runs, store, usageread};
use crate::app::Shared;
use crate::tasks::ops::{self, sql};

/// How often a kind is asked while a run of it works on a task or its last ask failed, and otherwise.
const RUNNING_EVERY: i64 = 600;
const IDLE_EVERY: i64 = 3600;
/// The least time between two asks of a kind: tasks ending one after another ask once.
const LEAST_GAP: i64 = 60;
/// The longest a usage command may take, the most of its output read, and of its error output kept.
const MOST_TIME: Duration = Duration::from_secs(60);
const MOST_OUT: u64 = 1 << 20;
const MOST_ERRORS: usize = 4096;
/// The `meta` key the last readings are kept under.
const KEY: &str = "subscriptions";

/// What is known of asking each kind; a restart asks anew.
#[derive(Default)]
pub struct Asking {
    kinds: Mutex<BTreeMap<String, Kind>>,
    /// the store's readings were read: none is saved before, so none read is lost
    loaded: AtomicBool,
}

#[derive(Default)]
struct Kind {
    /// when it was last asked
    asked: Option<i64>,
    /// its command runs now
    busy: bool,
    /// a task of it ended since it was last asked
    ended: bool,
    /// its live runs' open tasks at the last beat, by run
    open: HashMap<String, usize>,
    /// the command it was last asked by: a changed one asks at once
    line: String,
}

/// Whether a kind last asked at `asked` is due at `now`: often while a run of it works or its last ask failed,
/// soon after one of its tasks ended.
fn due(asked: Option<i64>, now: i64, often: bool, ended: bool) -> bool {
    let Some(asked) = asked else { return true };
    let gap = now - asked;
    gap >= LEAST_GAP && (ended || gap >= if often { RUNNING_EVERY } else { IDLE_EVERY })
}

/// Whether a task ended since the last beat: a run that had open tasks has fewer, or is gone.
fn ended(was: &HashMap<String, usize>, now: &HashMap<String, usize>) -> bool {
    was.iter().filter(|(_, n)| **n > 0).any(|(run, n)| now.get(run).is_none_or(|m| m < n))
}

impl Kind {
    /// A beat at `now` with its live runs' `open` tasks, its command `line` if any, and whether its last ask
    /// `failed`: whether to ask it now, marked busy then.
    fn step(&mut self, open: HashMap<String, usize>, line: Option<&str>, failed: bool, now: i64) -> bool {
        let was = std::mem::replace(&mut self.open, open);
        // kept until an ask starts: a task that ends during an ask, or too soon after one, is asked for later
        self.ended |= ended(&was, &self.open);
        let Some(line) = line else { return false };
        let asked = self.asked.filter(|_| self.line == line);
        let often = failed || self.open.values().any(|n| *n > 0);
        if self.busy || !due(asked, now, often, self.ended) {
            return false;
        }
        (self.busy, self.ended, self.asked, self.line) = (true, false, Some(now), line.to_string());
        true
    }
}

/// A kind's ask under way: its end, a panic too, frees the kind.
struct Busy(Shared, String);

impl Drop for Busy {
    fn drop(&mut self) {
        if let Some(k) = self.0.dispatcher.asking.kinds.lock().get_mut(&self.1) {
            k.busy = false;
        }
    }
}

/// The store's readings into `readings`, once, before any is saved (`loaded`).
fn load_once(tx: &Transaction, loaded: &AtomicBool, readings: &Readings) -> Result<(), String> {
    if !loaded.load(Ordering::SeqCst) {
        let kept: Option<String> = tx.query_row("SELECT value FROM meta WHERE key = ?1", [KEY], |r| r.get(0)).optional().map_err(sql)?;
        if let Some(Ok(v)) = kept.map(|t| serde_json::from_str::<Value>(&t)) {
            readings.load(&v);
        }
        loaded.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// Each walk's beat: asks every kind that is due, each in a task of its own; the walk does not wait for them.
pub async fn beat(app: &Shared) {
    let a = app.clone();
    let Ok((kinds, live)) = store(app, move |tx| {
        load_once(tx, &a.dispatcher.asking.loaded, &a.dispatcher.subscriptions)?;
        let kinds = ops::rows(tx, "SELECT DISTINCT kind FROM agents", [], |r| r.get::<_, String>(0))?;
        let open = format!("SELECT r.id, r.kind, (SELECT count(*) FROM run_tasks m JOIN tasks x ON x.id = m.task WHERE m.run = r.id AND x.status NOT IN ('DONE', 'FAILED'))
                            FROM runs r WHERE {}", runs::STILL);
        Ok((kinds, ops::rows(tx, &open, [], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as usize)))?))
    }).await else { return };
    let now = ops::now();
    for kind in kinds {
        let open: HashMap<String, usize> = live.iter().filter(|(_, k, _)| *k == kind).map(|(run, _, n)| (run.clone(), *n)).collect();
        let line = app.looks.settings.read().usage_commands.get(&kind).filter(|l| !l.is_empty()).cloned();
        let failed = app.dispatcher.subscriptions.has_failed(&kind);
        if !app.dispatcher.asking.kinds.lock().entry(kind.clone()).or_default().step(open, line.as_deref(), failed, now) {
            continue;
        }
        let (busy, line) = (Busy(app.clone(), kind.clone()), line.unwrap_or_default());
        tokio::spawn(async move {
            let app = busy.0.clone();
            let got = ask(&kind, &line, &crate::config::app_dir().join("usage"), MOST_TIME).await;
            drop(busy);
            let at = ops::now();
            match got {
                Ok(windows) => {
                    if app.dispatcher.subscriptions.heard(&kind, windows, at, Via::Usage) {
                        told(&app);
                    }
                }
                Err(why) => {
                    tracing::warn!(event = "dispatcher.usage", kind = %kind, error = %why);
                    app.dispatcher.subscriptions.failed(&kind, why, at);
                    tell(&app);   // a failure is not kept: nothing to save
                }
            }
        });
    }
}

/// Pages are told the readings as they are.
fn tell(app: &Shared) {
    app.pages.tell(&json!({"t": "subscriptions", "subscriptions": app.dispatcher.subscriptions.shown()}).to_string());
}

/// A reading changed: pages are told at once, and the store keeps the readings, as they are when it writes.
pub fn told(app: &Shared) {
    tell(app);
    let app = app.clone();
    tokio::spawn(async move {
        let a = app.clone();
        let saved = store(&app, move |tx| {
            let (asking, readings) = (&a.dispatcher.asking, &a.dispatcher.subscriptions);
            load_once(tx, &asking.loaded, readings)?;
            tx.execute("INSERT OR REPLACE INTO meta (key, value) VALUES (?1, ?2)", [KEY, &readings.kept().to_string()]).map(|_| ()).map_err(sql)
        }).await;
        if let Err(e) = saved {
            tracing::warn!(event = "dispatcher.usage.save", error = %e);
        }
    });
}

/// The PATH a usage command gets: terminumd's, then the folders the agents' installers use (launchd's is short).
fn path() -> String {
    let home = crate::config::home().display().to_string();
    let mut dirs: Vec<String> = std::env::var("PATH").unwrap_or_default().split(':').filter(|d| !d.is_empty()).map(str::to_string).collect();
    for known in crate::hosts::agentsinstall::KNOWN.map(|d| d.replace("$HOME", &home)) {
        // one `*` (nvm's node versions): each folder it stands for
        let found: Vec<String> = match known.split_once("/*/") {
            Some((parent, tail)) => std::fs::read_dir(parent).into_iter().flatten().flatten().map(|e| format!("{}/{tail}", e.path().display())).collect(),
            None => vec![known],
        };
        dirs.extend(found.into_iter().filter(|d| !d.contains('*')));
    }
    let mut seen = std::collections::HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs.join(":")
}

/// Runs `kind`'s usage command `line` through `sh -c`, in folder `dir` (one of its own), outside tmux, its process
/// group ended once it answered or after `most`; its windows, or why there are none.
pub async fn ask(kind: &str, line: &str, dir: &std::path::Path, most: Duration) -> Result<Vec<Window>, String> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let codex = kind == "codex";
    let mut cmd = tokio::process::Command::new("/bin/sh");
    cmd.arg("-c").arg(line).current_dir(dir).env("PATH", path()).env_remove("TMUX").env_remove("TMUX_PANE")
        .stdin(if codex { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped())
        .process_group(0).kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("cannot start /bin/sh: {e}"))?;
    let group = child.id().map(|p| p as libc::pid_t);
    // its error output read all along, its start kept: a full pipe would stop it before it answers
    let mut stderr = child.stderr.take().expect("piped");
    let errors = tokio::spawn(async move {
        let (mut kept, mut buf) = (Vec::new(), [0u8; 4096]);
        while let Ok(n @ 1..) = stderr.read(&mut buf).await {
            let room = MOST_ERRORS - kept.len();
            kept.extend_from_slice(&buf[..n.min(room)]);
        }
        String::from_utf8_lossy(&kept).into_owned()
    });
    let out = child.stdout.take().expect("piped");
    let mut input = child.stdin.take();
    let got = tokio::time::timeout(most, async {
        match input.as_mut() {
            Some(input) => codex_answer(input, out).await,
            None => claude_text(out).await,
        }
    }).await;
    drop(input);
    // a failure's exit code says why: a moment to end by itself; then what is left of its group goes
    let status = match &got {
        Ok(Err(_)) => tokio::time::timeout(Duration::from_secs(2), child.wait()).await.ok().and_then(Result::ok),
        _ => None,
    };
    end(&mut child, group).await;
    let errors = tokio::time::timeout(Duration::from_secs(1), errors).await.ok().and_then(Result::ok).unwrap_or_default();
    match got {
        Ok(Ok(w)) => Ok(w),
        Ok(Err(why)) => Err(failed(status.and_then(|s| s.code()), &errors, why)),
        Err(_) => Err(format!("no answer in {} s", most.as_secs_f64())),
    }
}

/// `codex app-server`'s answer: the question written while its input is open, its output read up to the answer.
async fn codex_answer(input: &mut ChildStdin, out: ChildStdout) -> Result<Vec<Window>, String> {
    input.write_all(usageread::CODEX_ASK.as_bytes()).await.map_err(|e| format!("its input closed: {e}"))?;
    let mut lines = BufReader::new(out.take(MOST_OUT)).lines();
    while let Ok(Some(l)) = lines.next_line().await {
        if let Ok(v) = serde_json::from_str::<Value>(&l) && v["id"] == 2 && v.get("method").is_none() {
            return usageread::of_codex(&v);
        }
    }
    Err("it ended without an answer".into())
}

/// `claude -p /usage`'s whole output, read.
async fn claude_text(out: ChildStdout) -> Result<Vec<Window>, String> {
    let mut text = String::new();
    BufReader::new(out.take(MOST_OUT)).read_to_string(&mut text).await.map_err(|e| format!("its output: {e}"))?;
    usageread::of_claude(&text)
}

/// Its process group ended, all of it at once (a failure's leader was waited for above: the group's number stays
/// its own while any of it lives, and pids are not reused within seconds).
async fn end(child: &mut Child, group: Option<libc::pid_t>) {
    if let Some(g) = group {
        // SAFETY: killpg has no preconditions; `g` is the group made for this command
        unsafe { libc::killpg(g, libc::SIGKILL) };
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
}

/// Why a usage command gave no windows: its exit and its error output's first line when it failed, else `why`.
fn failed(code: Option<i32>, errors: &str, why: String) -> String {
    match (code, usageread::first_line(errors, 160)) {
        (Some(127), _) => "its program was not found on terminumd's PATH: set dispatcher.usageCommands with its full path".into(),
        (Some(code), Some(line)) if code != 0 => format!("it ended with exit {code}: {line}"),
        (Some(code), None) if code != 0 => format!("it ended with exit {code}"),
        _ => why,
    }
}

#[cfg(test)]
#[path = "usageask_tests.rs"]
mod tests;
