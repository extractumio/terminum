// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Per-pane workspace: tree state + open tabs, keyed by the bridge's state key
//! (iTerm2 session id, or tmux pane). Persisted to JSON, debounced, atomic rename.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::events::{Bus, Event};

#[derive(Clone, Serialize, Deserialize, Debug, Default, PartialEq)]
pub struct Tab {
    pub path: String,
    #[serde(default = "default_view")]
    pub view: String,
}

fn default_view() -> String {
    "auto".into()
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
pub struct Pane {
    #[serde(default)]
    pub rev: u64,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub expanded: Vec<String>,
    #[serde(default)]
    pub selected: Vec<String>,
    #[serde(default)]
    pub scroll: f64,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub active_tab: Option<usize>,
    #[serde(default)]
    pub updated: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    version: u32,
    #[serde(default)]
    prefs: Value,
    #[serde(default)]
    panes: HashMap<String, Pane>,
}

/// A failed write is tried again after this long, doubling up to `RETRY_MAX`.
const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// The views a tab can be in (ui/src/api.ts `Tab`).
const VIEWS: [&str; 3] = ["auto", "rendered", "source"];

/// Why a PUT was not applied.
#[derive(Debug)]
pub enum Refused {
    /// another revision is current (it is the answer)
    Stale(Pane),
    /// not a pane's state: what is wrong
    Bad(String),
}

pub struct Store {
    data: Mutex<File>,
    writing: Mutex<()>,
    /// the last write's error while the file lags what was accepted (said to every panel)
    unsaved: Mutex<Option<String>>,
    path: PathBuf,
    dirty: Notify,
    bus: Bus,
    ttl: u64,
}

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl Store {
    pub fn load(path: PathBuf, ttl_days: u64, bus: Bus) -> Arc<Self> {
        let mut data = match fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<File>(&bytes) {
                Ok(f) if f.version == 1 => f,
                _ => {
                    let bak = path.with_extension("json.bak");
                    let _ = fs::rename(&path, &bak);
                    tracing::warn!(event = "workspace.reset", reason = "parse_error", backup = %bak.display());
                    File::default()
                }
            },
            Err(_) => File::default(),
        };
        data.version = 1;
        // Wait for authoritative liveness before pruning: a live inactive pane may
        // predate the TTL when fbd restarts.
        let store = Arc::new(Store { data: Mutex::new(data), writing: Mutex::new(()), unsaved: Mutex::new(None), path,
            dirty: Notify::new(), bus, ttl: ttl_days.saturating_mul(86400) });
        tokio::spawn(store.clone().write_on_change());
        store
    }

    /// Writes 500 ms after a change; a failed write is said and tried again with back-off
    /// until one works (a change meanwhile waits for it).
    async fn write_on_change(self: Arc<Self>) {
        let mut wait = RETRY_FIRST;
        loop {
            self.dirty.notified().await;
            tokio::time::sleep(Duration::from_millis(500)).await;
            let s = self.clone();
            let done = tokio::task::spawn_blocking(move || s.flush()).await.unwrap_or_else(|e| Err(e.to_string()));
            if done.is_ok() {
                wait = RETRY_FIRST;
                continue;
            }
            tokio::time::sleep(wait).await;
            wait = (wait * 2).min(RETRY_MAX);
            self.dirty.notify_one();
        }
    }

    /// Writes the file now; its outcome is kept and said to the panels when it changes.
    pub fn flush(&self) -> Result<(), String> {
        // Shutdown and the debounce worker must serialize snapshot capture and publication.
        let _writing = self.writing.lock();
        let bytes = serde_json::to_vec(&*self.data.lock()).unwrap_or_default(); // lock held only to serialize
        let done = crate::settings::write_atomic(&self.path, &bytes).map_err(|e| e.to_string());
        if let Err(e) = &done {
            tracing::warn!(event = "workspace.write_failed", path = %self.path.display(), error = %e);
        }
        let error = done.clone().err();
        let mut unsaved = self.unsaved.lock();
        if *unsaved != error {
            *unsaved = error.clone();
            self.bus.send(Event::Unsaved { error });
        }
        done
    }

    /// The last write's error while the file lags what was accepted.
    pub fn unsaved(&self) -> Option<String> {
        self.unsaved.lock().clone()
    }

    fn touched(&self, key: &str, rev: u64, by: Option<String>) {
        self.dirty.notify_one();
        self.bus.send(Event::Workspace { key: key.to_string(), rev, by });
    }

    pub fn get(&self, key: &str, cwd: Option<&str>) -> Pane {
        let data = self.data.lock();
        match data.panes.get(key) {
            Some(p) => p.clone(),
            None => Pane { root: cwd.unwrap_or_default().to_string(), ..Default::default() },
        }
    }

    /// The bridge reports the pane's cwd. A new root clears the tree state but keeps tabs;
    /// true when the root changed.
    pub fn set_root(&self, key: &str, root: &str) -> bool {
        let rev = {
            let mut data = self.data.lock();
            let p = data.panes.entry(key.to_string()).or_default();
            if p.root == root {
                return false;
            }
            p.root = root.to_string();
            p.expanded.clear();
            p.selected.clear();
            p.scroll = 0.0;
            p.rev += 1;
            p.updated = now();
            p.rev
        };
        self.touched(key, rev, None);
        true
    }

    /// Replace a pane's state if the client saw the latest revision (else the current one) and
    /// it is a pane's state. The new rev, and whether what is watched (root, expanded folders,
    /// tabs) changed.
    pub fn put(&self, key: &str, mut pane: Pane, by: Option<String>) -> Result<(u64, bool), Refused> {
        checked(&pane).map_err(Refused::Bad)?;
        let (rev, watched) = {
            let mut data = self.data.lock();
            let cur = data.panes.entry(key.to_string()).or_default();
            if pane.rev != cur.rev || (!cur.root.is_empty() && pane.root != cur.root) {
                return Err(Refused::Stale(cur.clone()));
            }
            let watched = pane.root != cur.root || pane.expanded != cur.expanded
                || !pane.tabs.iter().map(|t| &t.path).eq(cur.tabs.iter().map(|t| &t.path));
            pane.rev = cur.rev + 1;
            pane.updated = now();
            *cur = pane;
            (cur.rev, watched)
        };
        self.touched(key, rev, by);
        Ok((rev, watched))
    }

    pub fn prefs(&self) -> Value {
        self.data.lock().prefs.clone()
    }

    /// The panel's preferences: an object (or null, none).
    pub fn set_prefs(&self, prefs: Value) -> Result<(), String> {
        if !(prefs.is_object() || prefs.is_null()) {
            return Err("Preferences are an object".into());
        }
        self.data.lock().prefs = prefs;
        self.dirty.notify_one();
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.data.lock().panes.len()
    }

    /// Coarse liveness persistence does not change content revisions or emit updates.
    pub fn maintain(&self, keys: &HashSet<String>, protect_tmux: bool) {
        self.maintain_at(keys, protect_tmux, now());
    }

    fn maintain_at(&self, keys: &HashSet<String>, protect_tmux: bool, at: u64) {
        let mut data = self.data.lock();
        let before = data.panes.len();
        let mut touched = false;
        let cutoff = at.saturating_sub(self.ttl);
        data.panes.retain(|key, pane| {
            if keys.contains(key) {
                if at.saturating_sub(pane.updated) >= 3600 {
                    pane.updated = at;
                    touched = true;
                }
                true
            } else {
                pane.updated >= cutoff || (protect_tmux && key.starts_with("tmux:"))
            }
        });
        if touched || before != data.panes.len() {
            self.dirty.notify_one();
        }
    }
}

/// A pane's state as a panel may write it: an absolute root (or none yet), tabs in a known
/// view, the active tab one of them.
fn checked(p: &Pane) -> Result<(), String> {
    if !p.root.is_empty() && !p.root.starts_with('/') {
        return Err(format!("The root is not a full path: {}", p.root));
    }
    if let Some(t) = p.tabs.iter().find(|t| !VIEWS.contains(&t.view.as_str())) {
        return Err(format!("A tab's view is one of {}: {}", VIEWS.join(", "), t.view));
    }
    if p.active_tab.is_some_and(|a| a >= p.tabs.len()) {
        return Err(format!("The active tab is not one of the {} tabs", p.tabs.len()));
    }
    Ok(())
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod more_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_waiting_flush_captures_the_latest_snapshot_after_the_writer_lock() {
        let path = crate::testutil::temp_root().join(format!("fbd-ws-flush-{}.json", std::process::id()));
        let s = Arc::new(Store { data: Mutex::new(File { version: 1, ..Default::default() }),
            writing: Mutex::new(()), unsaved: Mutex::new(None), path: path.clone(), dirty: Notify::new(), bus: Bus::new(), ttl: 14 * 86400 });
        let write = s.writing.lock();
        let (started, waiting) = std::sync::mpsc::channel();
        let thread = {
            let s = s.clone();
            std::thread::spawn(move || { started.send(()).unwrap(); s.flush().unwrap(); })
        };
        waiting.recv().unwrap();
        s.data.lock().prefs = serde_json::json!({"newest": "large snapshot".repeat(1000)});
        drop(write);
        thread.join().unwrap();
        let saved: File = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved.prefs, s.prefs());
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn ongoing_ttl_preserves_live_idle_panes_and_uncertain_tmux_without_revisions() {
        let path = crate::testutil::temp_root().join(format!("fbd-ws-ttl-{}.json", std::process::id()));
        fs::write(&path, r#"{"version":1,"panes":{
            "live":{"root":"/a","updated":1,"rev":7},
            "closed":{"root":"/b","updated":1},
            "tmux:devbox:/tmp/t:%1":{"root":"/c","updated":1}}}"#).unwrap();
        let s = Store::load(path.clone(), 14, Bus::new());
        assert_eq!(s.len(), 3, "startup waits for live inventory");
        let at = 15 * 86400;
        let keys = HashSet::from(["live".into()]);
        s.maintain_at(&keys, true, at);
        assert_eq!(s.len(), 2, "closed state purged without restarting");
        let live = s.get("live", None);
        assert_eq!((live.updated, live.rev), (at, 7));
        s.maintain_at(&keys, true, at + 5);
        assert_eq!(s.get("live", None).updated, at, "no five-second disk writes");
        s.maintain_at(&keys, false, at + 10);
        assert_eq!(s.len(), 1, "resolved absence permits tmux TTL cleanup");
        s.flush().unwrap();
        let loaded = Store::load(path.clone(), 14, Bus::new());
        assert_eq!(loaded.len(), 1);
        let _ = fs::remove_file(path);
    }

    /// AC-35: a build reads state written by a newer one (unknown fields) or an older one
    /// (fields missing), so a rollback or upgrade never loses the workspaces.
    #[tokio::test]
    async fn state_of_other_builds_is_readable() {
        let path = crate::testutil::temp_root().join(format!("fbd-ws-compat-{}.json", std::process::id()));
        fs::write(&path, r#"{"version": 1, "from_a_newer_build": true, "prefs": {"hidden": false},
            "panes": {"p1": {"root": "/a", "updated": 4102444800, "tabs": [{"path": "/a/x.rs", "pinned": true}], "zoom": 2}},
            "older": {}}"#).unwrap();
        let s = Store::load(path.clone(), 14, Bus::new());
        let p = s.get("p1", None);
        assert_eq!((p.root.as_str(), p.tabs[0].path.as_str(), p.tabs[0].view.as_str()), ("/a", "/a/x.rs", "auto"));
        assert!(p.expanded.is_empty() && p.active_tab.is_none(), "missing fields take their defaults");
        assert_eq!(s.prefs()["hidden"], false);
        assert!(!path.with_extension("json.bak").exists(), "not treated as corrupt");
        let _ = fs::remove_file(&path);
    }

    #[tokio::test]
    async fn cd_clears_tree_keeps_tabs_and_rev_guards() {
        let path = crate::testutil::temp_root().join(format!("fbd-ws-{}.json", std::process::id()));
        let s = Store::load(path.clone(), 14, Bus::new());
        s.set_root("p1", "/a");
        let mut p = s.get("p1", None);
        assert_eq!((p.root.as_str(), p.rev), ("/a", 1));
        p.expanded = vec!["/a/src".into()];
        p.selected = vec!["/a/src/x.rs".into()];
        p.tabs = vec![Tab { path: "/a/README.md".into(), view: "rendered".into() }];
        assert_eq!(s.put("p1", p.clone(), None).ok(), Some((2, true)), "folders and tabs: watched");
        p.rev = 2;
        p.scroll = 40.0;
        assert_eq!(s.put("p1", p.clone(), None).ok(), Some((3, false)), "a scroll: nothing watched changed");
        // a second panel still holding an older rev is refused
        assert!(matches!(s.put("p1", p.clone(), None), Err(Refused::Stale(cur)) if cur.rev == 3));
        s.set_root("p1", "/b");
        let p = s.get("p1", None);
        assert!(p.expanded.is_empty() && p.selected.is_empty());
        assert_eq!(p.tabs.len(), 1);
        assert_eq!(p.root, "/b");
        s.flush().unwrap();
        let s2 = Store::load(path.clone(), 14, Bus::new());
        assert_eq!(s2.get("p1", None).tabs.len(), 1);
        fs::write(&path, "{not json").unwrap();
        let s3 = Store::load(path.clone(), 14, Bus::new());
        assert_eq!(s3.len(), 0);
        assert!(path.with_extension("json.bak").exists());
        let _ = fs::remove_file(path.with_extension("json.bak"));
    }
}
