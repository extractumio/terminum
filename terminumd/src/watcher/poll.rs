// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-61: recover late/missing native notifications without enumerating folder contents.
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Weak;

use tokio::sync::mpsc;

use super::{Watcher, BATCH};

pub(super) async fn run(weak: Weak<Watcher>, tx: mpsc::UnboundedSender<PathBuf>) {
    let mut previous = HashMap::new();
    loop {
        tokio::time::sleep(BATCH).await;
        let Some(watcher) = weak.upgrade() else { break; };
        let paths = watcher.paths.lock().clone();
        drop(watcher);
        let want: HashSet<_> = paths.dirs.union(&paths.files).cloned().collect();
        if want.is_empty() { previous.clear(); continue; }
        let Ok((next, changed)) = tokio::task::spawn_blocking(move || sample(previous, want)).await else { break; };
        previous = next;
        let Some(watcher) = weak.upgrade() else { break; };
        {
            let current = watcher.paths.lock();
            previous.retain(|p, _| current.dirs.contains(p) || current.files.contains(p));
        }
        drop(watcher);
        for path in changed {
            if !previous.contains_key(&path) { continue; }
            tracing::debug!(event = "watch.reconcile", path = %path.display());
            if tx.send(path).is_err() { return; }
        }
    }
}

fn sample(mut previous: HashMap<PathBuf, Option<String>>, want: HashSet<PathBuf>) -> (HashMap<PathBuf, Option<String>>, Vec<PathBuf>) {
    previous.retain(|p, _| want.contains(p));
    let mut changed = Vec::new();
    for path in want {
        let stamp = match std::fs::metadata(&path) {
            Ok(meta) => Some(crate::files::etag(&meta)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => {
                tracing::debug!(event = "watch.metadata_failed", path = %path.display(), error = %e);
                continue;
            }
        };
        // A first sample also reconciles edits made before the tab's workspace save.
        if previous.get(&path) != Some(&stamp) { changed.push(path.clone()); }
        previous.insert(path, stamp);
    }
    (previous, changed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_errors_are_not_deletions_and_removed_paths_leave_the_snapshot() {
        let bad = PathBuf::from("bad\0path");
        let previous = HashMap::from([(bad.clone(), Some("keep".into()))]);
        let (next, changed) = sample(previous, HashSet::from([bad.clone()]));
        assert!(changed.is_empty());
        assert_eq!(next[&bad].as_deref(), Some("keep"));
        let (next, changed) = sample(next, HashSet::new());
        assert!(next.is_empty() && changed.is_empty());
    }
}
