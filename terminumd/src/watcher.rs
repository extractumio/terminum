// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Live refresh (AC-61): native notifications plus metadata reconciliation of the watched
//! folders and open files, since FSEvents can arrive seconds late. Both feed one 500 ms batch.

mod poll;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{EventKind, RecursiveMode};
use parking_lot::Mutex;
use tokio::sync::mpsc;

use crate::events::{publish_change, Bus};
use crate::listing::Cache;

const BATCH: Duration = Duration::from_millis(500);

pub struct Watcher {
    inner: Mutex<(notify::RecommendedWatcher, HashSet<PathBuf>)>,
    paths: Mutex<Paths>,
}

#[derive(Clone, Default)]
pub struct Paths {
    pub dirs: HashSet<PathBuf>,
    pub files: HashSet<PathBuf>,
}

impl Paths {
    pub fn extend(&mut self, other: Self) {
        self.dirs.extend(other.dirs);
        self.files.extend(other.files);
    }
}

impl Watcher {
    pub fn start(cache: Arc<Cache>, bus: Bus) -> notify::Result<Arc<Self>> {
        // A private debug test can reproduce macOS withholding otherwise valid callbacks.
        let delay = if cfg!(debug_assertions) { std::env::var("TM_TEST_WATCH_DELAY_MS").ok().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0) } else { 0 };
        let runtime = tokio::runtime::Handle::current();
        Self::start_with(cache, bus, |tx| notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                tracing::debug!(event = "watch.raw", kind = ?ev.kind, paths = ?ev.paths, rescan = ev.need_rescan());
                if !matches!(ev.kind, EventKind::Access(_)) {
                    if delay == 0 { for p in ev.paths { let _ = tx.send(p); } }
                    else {
                        let tx = tx.clone();
                        runtime.spawn(async move {
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                            for p in ev.paths { let _ = tx.send(p); }
                        });
                    }
                }
            }
        }))
    }

    fn start_with(cache: Arc<Cache>, bus: Bus, listen: impl FnOnce(mpsc::UnboundedSender<PathBuf>) -> notify::Result<notify::RecommendedWatcher>) -> notify::Result<Arc<Self>> {
        let (tx, mut rx) = mpsc::unbounded_channel::<PathBuf>();
        let w = listen(tx.clone())?;
        let me = Arc::new(Watcher { inner: Mutex::new((w, HashSet::new())), paths: Mutex::new(Paths::default()) });
        tokio::spawn(poll::run(Arc::downgrade(&me), tx));
        let watched = Arc::downgrade(&me);
        tokio::spawn(async move {
            while let Some(first) = rx.recv().await {
                let mut batch: HashSet<PathBuf> = HashSet::from([first]);
                let deadline = tokio::time::Instant::now() + BATCH;
                while let Ok(Some(p)) = tokio::time::timeout_at(deadline, rx.recv()).await {
                    batch.insert(p);
                }
                let Some(watched) = watched.upgrade() else { break; };
                let dirs_watched = watched.paths.lock().dirs.clone();
                drop(watched);
                let mut dirs = HashSet::new();
                let mut files = HashSet::new();
                for p in batch {
                    // FSEvents is recursive underneath; keep only direct children of watched folders
                    if let Some(parent) = p.parent().filter(|d| dirs_watched.contains(*d)) {
                        dirs.insert(parent.to_path_buf());
                        files.insert(p.clone());
                    }
                    if dirs_watched.contains(&p) {
                        dirs.insert(p);
                    }
                }
                if dirs.is_empty() {
                    continue;
                }
                tracing::debug!(event = "fs.change", dirs = dirs.len(), files = files.len());
                let (dirs, files): (Vec<_>, Vec<_>) = (dirs.into_iter().collect(), files.into_iter().collect());
                let (cache, bus) = (cache.clone(), bus.clone());
                if let Err(e) = tokio::task::spawn_blocking(move || publish_change(&cache, &bus, &dirs, &files, &[])).await {
                    tracing::warn!(event = "watch.publish_failed", error = %e);
                }
            }
        });
        Ok(me)
    }

    /// Replace the watched set (only the differences are applied).
    pub fn set(&self, want: Paths) {
        let mut g = self.inner.lock();
        let (w, have) = &mut *g;
        let changed = *have != want.dirs;
        if changed { tracing::debug!(event = "watch.set", have = ?have, want = ?want.dirs); }
        set_watched(w, have, want.dirs.clone());
        if changed { tracing::debug!(event = "watch.ready", dirs = have.len(), files = want.files.len()); }
        *self.paths.lock() = want;
    }

    pub fn len(&self) -> usize {
        self.inner.lock().1.len()
    }
}

fn set_watched(w: &mut impl notify::Watcher, have: &mut HashSet<PathBuf>, want: HashSet<PathBuf>) {
    for p in have.difference(&want) {
        let _ = w.unwatch(p);
    }
    have.retain(|p| want.contains(p));
    for p in want {
        if have.contains(&p) { continue; }
        match w.watch(&p, RecursiveMode::NonRecursive) {
            Ok(()) => { have.insert(p); }
            Err(e) => tracing::debug!(event = "watch.failed", path = %p.display(), error = %e),
        }
    }
}

/// Folders to watch for one pane: its root, expanded folders and the folders of open tabs.
pub fn wanted(root: &str, expanded: &[String], tabs: impl Iterator<Item = String>) -> Paths {
    let mut s: HashSet<PathBuf> = HashSet::new();
    if !root.is_empty() {
        s.insert(PathBuf::from(root));
    }
    s.extend(expanded.iter().map(PathBuf::from));
    let files: HashSet<PathBuf> = tabs.map(PathBuf::from).collect();
    s.extend(files.iter().filter_map(|t| t.parent().map(Path::to_path_buf)));
    Paths { dirs: s, files }
}

#[cfg(test)]
mod recovery_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Fake { calls: usize, unwatched: usize }

    impl notify::Watcher for Fake {
        fn new<F: notify::EventHandler>(_: F, _: notify::Config) -> notify::Result<Self> { Ok(Self::default()) }
        fn watch(&mut self, _: &Path, _: RecursiveMode) -> notify::Result<()> {
            self.calls += 1;
            if self.calls == 1 { Err(notify::Error::generic("temporary quota")) } else { Ok(()) }
        }
        fn unwatch(&mut self, _: &Path) -> notify::Result<()> { self.unwatched += 1; Ok(()) }
        fn kind() -> notify::WatcherKind { notify::WatcherKind::NullWatcher }
    }

    #[test]
    fn failed_subscriptions_are_retried_without_changing_the_wanted_set() {
        let (mut watcher, mut have) = (Fake::default(), HashSet::new());
        let want = HashSet::from([PathBuf::from("/sandbox/project")]);
        set_watched(&mut watcher, &mut have, want.clone());
        assert!(have.is_empty(), "failed watches are not installed");
        set_watched(&mut watcher, &mut have, want.clone());
        assert_eq!((watcher.calls, &have), (2, &want));
        set_watched(&mut watcher, &mut have, want);
        assert_eq!(watcher.calls, 2, "a successful watch is not repeated");
        set_watched(&mut watcher, &mut have, HashSet::new());
        assert_eq!(watcher.unwatched, 1);
        assert!(have.is_empty());
    }
}
