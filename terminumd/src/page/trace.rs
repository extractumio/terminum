// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Latency trace (AC-38), ported from iterm-enhancer's bridge (web/trace.py) with tmux in
//! place of iTerm2. With `?trace=1` the page and terminumd record each key's way to its echo,
//! sessions opened, round trips and stalls into `logs/trace-<date>-<time>.jsonl` (0600, one
//! JSON record per line) for 15 minutes or 20 MB; the newest 10 files are kept. Nothing typed
//! or shown is recorded: a page record keeps only known names with numbers, booleans and
//! listed words, and a pane is recorded as `%N` with its mode (local or remote), never its
//! host. With `?trace=2` the page asks only for the key stages in its screen messages
//! (`stages`): nothing is written then. A write that fails (a full disk) ends the trace with
//! a line in the log; typing and screens go on.
//!
//! Key stages (in the frame's `keys` and the `key` record): `to_tmux_ms` (handed to tmux,
//! a `step` of the caller), `until_change_ms` (until the capture that showed it began),
//! `polls` (captures that found no change), `woken` (key|output|poll: what woke that capture),
//! `capture_ms`, `encode_ms` (cut, diff and encoding after the capture), `total_ms`, `skip`
//! (frames where the cursor neither moved nor its row changed), `moved`.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde_json::{Map, Value, json};

pub const TRACE_FOR: Duration = Duration::from_secs(15 * 60);
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;
pub const MAX_BATCH: usize = 500;
pub const KEEP_FILES: usize = 10;
/// How often the client calls `Tracer::tick`.
pub const EVERY: Duration = Duration::from_secs(2);
/// Between tmux round trips (`display-message -p x`): they share the channel with send-keys.
pub const PROBE_EVERY: Duration = Duration::from_secs(6);
const LAG_EVERY: Duration = Duration::from_millis(100);
const LAG_NOTE: f64 = 20.0;
const SLOW_MSG: f64 = 20.0;
const STALE: Duration = Duration::from_secs(10);

/// Words a page record may hold, by field.
const WORDS: [(&str, &[&str]); 5] = [
    ("path", &["keydown", "input", "compose", "hotkey", "paste", "tap", "line", "other"]),
    ("cls", &["char", "text", "enter", "backspace", "tab", "esc", "seq", "ctrl", "none"]),
    ("via", &["text", "keys", "paste"]),
    ("woken", &["first", "next", "key", "output", "poll"]),
    ("what", &["screen", "hist"]),
];
/// What a page may record: event -> its fields (numbers or booleans unless in WORDS or "sid").
const PAGE: [(&str, &[&str]); 6] = [
    ("echo", &["k", "path", "cls", "n", "comp", "in", "buf", "rt", "parse", "draw", "paint", "total", "via", "woken",
        "to_tmux_ms", "until_change_ms", "polls", "capture_ms", "encode_ms", "total_ms", "skip", "moved"]),
    ("noecho", &["k", "path", "cls", "n"]),
    ("open", &["sid", "first", "paint", "bytes"]),
    ("ping", &["n", "rtt"]),
    ("jank", &["count", "max"]),
    ("draw", &["what", "ms", "rows"]),
];
static CLIENTS: AtomicU64 = AtomicU64::new(1);

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 100_000.0).round() / 100.0
}

fn since(t: Instant) -> f64 {
    ms(t.elapsed())
}

/// The pane of a state key (`tmux:<host>:<socket>:%N` -> `%N`): never the host's name.
pub fn pane_of(sid: &str) -> Option<String> {
    let p = sid.rsplit(':').next()?;
    let digits = p.strip_prefix('%')?;
    ((1..=9).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_digit())).then(|| p.to_string())
}

/// A page record with only what it may hold, or None.
pub fn clean(rec: &Value) -> Option<Map<String, Value>> {
    let ev = rec.get("ev")?.as_str()?;
    let (_, fields) = PAGE.iter().find(|(e, _)| *e == ev)?;
    let mut out = Map::new();
    out.insert("ev".into(), json!(ev));
    for &name in *fields {
        let Some(v) = rec.get(name) else { continue };
        let kept = if let Some((_, words)) = WORDS.iter().find(|(w, _)| *w == name) {
            v.as_str().filter(|s| words.contains(s)).map(|s| json!(s))
        } else if name == "sid" {
            v.as_str().and_then(pane_of).map(Value::String)
        } else if v.is_boolean() {
            Some(v.clone())
        } else {
            v.as_f64().filter(|n| n.is_finite() && n.abs() < 1e9).map(|n| {
                if v.is_i64() || v.is_u64() { v.clone() } else { json!((n * 100.0).round() / 100.0) }
            })
        };
        if let Some(k) = kept {
            out.insert(name.into(), k);
        }
    }
    Some(out)
}

/// `trace-YYYYMMDD-HHMMSS.jsonl` in local time.
#[allow(deprecated)] // the libc crate marks musl's time_t deprecated ahead of its move to 64 bits
fn file_name() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) as libc::time_t;
    // SAFETY: localtime_r writes only into the zeroed tm it is given.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&now, &mut tm) };
    format!("trace-{:04}{:02}{:02}-{:02}{:02}{:02}.jsonl", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min, tm.tm_sec)
}

struct Inner {
    file: Option<Box<dyn Write + Send>>,
    path: Option<PathBuf>,
    until: Instant,
    size: u64,
    /// Counts starts, so a page knows the trace it joined has ended.
    run: u64,
}

/// The trace file, shared by every page: one at a time, ended on terminumd's clock.
pub struct Recorder {
    dir: PathBuf,
    max_bytes: u64,
    lasts: Duration,
    inner: Mutex<Inner>,
}

impl Recorder {
    /// `dir` is the logs folder (`~/.terminum/logs`).
    pub fn new(dir: PathBuf) -> Arc<Recorder> {
        Recorder::with_limits(dir, MAX_BYTES, TRACE_FOR)
    }

    pub fn with_limits(dir: PathBuf, max_bytes: u64, lasts: Duration) -> Arc<Recorder> {
        let inner = Inner { file: None, path: None, until: Instant::now(), size: 0, run: 0 };
        Arc::new(Recorder { dir, max_bytes, lasts, inner: Mutex::new(inner) })
    }

    fn live(i: &Inner, max: u64) -> bool {
        i.file.is_some() && Instant::now() < i.until && i.size < max
    }

    #[cfg(test)]
    pub fn on(&self) -> bool {
        Recorder::live(&self.inner.lock(), self.max_bytes)
    }

    /// The generation of the trace that is on, if one is.
    fn current(&self) -> Option<u64> {
        let i = self.inner.lock();
        Recorder::live(&i, self.max_bytes).then_some(i.run)
    }

    /// Seconds left, 0 when off.
    pub fn left(&self) -> u64 {
        let i = self.inner.lock();
        if Recorder::live(&i, self.max_bytes) { i.until.saturating_duration_since(Instant::now()).as_secs_f64().round() as u64 } else { 0 }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.inner.lock().path.clone()
    }

    /// A new file (the oldest beyond the newest 10 go); the lag watch runs while it is on.
    pub fn start(self: &Arc<Self>) -> std::io::Result<()> {
        self.stop();
        fs::create_dir_all(&self.dir)?;
        let mut old: Vec<PathBuf> = fs::read_dir(&self.dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("trace-") && n.ends_with(".jsonl")))
            .collect();
        old.sort();
        let drop = old.len().saturating_sub(KEEP_FILES - 1);
        for p in &old[..drop] {
            let _ = fs::remove_file(p);
        }
        let path = self.dir.join(file_name());
        let file = OpenOptions::new().create(true).append(true).mode(0o600).open(&path)?;
        let run = {
            let mut i = self.inner.lock();
            let run = i.run + 1;
            *i = Inner { file: Some(Box::new(file)), path: Some(path.clone()), until: Instant::now() + self.lasts, size: 0, run };
            run
        };
        tracing::info!(event = "trace.on", path = %path.display());
        if let Ok(rt) = tokio::runtime::Handle::try_current() {
            rt.spawn(watch(self.clone(), run));
        }
        Ok(())
    }

    pub fn stop(&self) {
        let mut i = self.inner.lock();
        if i.file.take().is_some() {
            tracing::info!(event = "trace.off", path = %i.path.as_ref().map_or(String::new(), |p| p.display().to_string()), bytes = i.size);
        }
    }

    /// One record, when the trace is on; a failed write ends the trace.
    pub fn write(&self, src: &str, rec: Map<String, Value>) {
        let mut i = self.inner.lock();
        if !Recorder::live(&i, self.max_bytes) {
            return;
        }
        let at = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        let mut all = Map::new();
        all.insert("at".into(), json!(at));
        all.insert("src".into(), json!(src));
        all.extend(rec);
        let line = Value::Object(all).to_string() + "\n";
        i.size += line.len() as u64;
        if i.size > self.max_bytes {
            drop(i);
            return self.stop();
        }
        let res = i.file.as_mut().map(|f| f.write_all(line.as_bytes()));
        if let Some(Err(e)) = res {
            drop(i);
            tracing::warn!(event = "trace.failed", msg = %e);
            self.stop();
        }
    }

    /// A record of terminumd's own (a list refresh, a stall), when the trace is on.
    pub fn note(&self, ev: &str, fields: Value) {
        let mut rec = Map::new();
        rec.insert("ev".into(), json!(ev));
        if let Value::Object(f) = fields {
            rec.extend(f);
        }
        self.write("terminumd", rec);
    }
}

/// While the trace `run` is on: the async workers' lateness noted; the file closed at its end.
async fn watch(rec: Arc<Recorder>, run: u64) {
    while rec.current() == Some(run) {
        let t = Instant::now();
        tokio::time::sleep(LAG_EVERY).await;
        let late = ms(t.elapsed().saturating_sub(LAG_EVERY));
        if late > LAG_NOTE {
            rec.note("lag", json!({"ms": late}));
        }
    }
    if rec.inner.lock().run == run {
        rec.stop();
    }
}

/// A key on its way: timed step by step (`Tracer::step`) until a frame shows it.
#[derive(Debug, Clone)]
pub struct KeyRec {
    k: i64,
    t0: Instant,
    m: Instant,
    fields: Map<String, Value>,
    polls: u32,
    skip: u32,
}

/// One page's part: its keys on their way, the pane it opens, the tmux round trip.
pub struct Tracer {
    rec: Arc<Recorder>,
    id: u64,
    joined: Option<u64>,
    stages: bool,
    keys: Vec<KeyRec>,
    woke: Option<&'static str>,
    read_at: Option<Instant>,
    read_ms: f64,
    read_woke: &'static str,
    mode: &'static str,
    opened: Option<(String, Instant)>,
    probed: Instant,
}

#[cfg(test)]
#[path = "trace_tests.rs"]
mod tests;

mod tracer;
