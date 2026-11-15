// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-61: real mutations with every native callback withheld, including late registration.
use super::*;
use crate::events::Event;

struct Sandbox(PathBuf);
impl Sandbox {
    fn new(tag: &str) -> Self {
        Self(crate::testutil::fresh_dir(tag).canonicalize().unwrap())
    }
}
impl Drop for Sandbox {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
}

async fn changed(rx: &mut tokio::sync::broadcast::Receiver<Event>, path: &Path, file: bool) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::FsChange { dirs, files, .. } = rx.recv().await.unwrap() {
                if file {
                    if let Some(s) = files.into_iter().find(|s| Path::new(&s.path) == path) { return s.etag; }
                } else if dirs.iter().any(|d| Path::new(d) == path) { return None; }
            }
        }
    }).await.expect("metadata reconciliation must recover without any native notification")
}

fn silent(cache: Arc<Cache>, bus: Bus) -> Arc<Watcher> {
    Watcher::start_with(cache, bus, |_| notify::recommended_watcher(|_: notify::Result<notify::Event>| {})).unwrap()
}

#[tokio::test]
async fn edits_deletion_and_recreation_recover_without_native_callbacks() {
    let sb = Sandbox::new("watch-edits");
    let path = sb.0.join("app.py");
    std::fs::write(&path, "print(1)\n").unwrap();
    let bus = Bus::new();
    let mut rx = bus.subscribe();
    let w = silent(Cache::new(1 << 20), bus);
    w.set(wanted(sb.0.to_str().unwrap(), &[], [path.display().to_string()].into_iter()));
    let first = changed(&mut rx, &path, true).await.unwrap();
    std::fs::write(&path, "print(1)\nprint(2)\n").unwrap();
    let edited = changed(&mut rx, &path, true).await.unwrap();
    assert_ne!(edited, first, "a clean tab can reload and a dirty tab can detect the external edit");
    std::fs::remove_file(&path).unwrap();
    assert_eq!(changed(&mut rx, &path, true).await, None, "deleted on disk");
    std::fs::write(&path, "recreated\n").unwrap();
    assert!(changed(&mut rx, &path, true).await.is_some());
}

#[tokio::test]
async fn folder_changes_invalidate_listings_without_native_callbacks() {
    let sb = Sandbox::new("watch-folder");
    let path = sb.0.join("new.rs");
    let cache = Cache::new(1 << 20);
    let bus = Bus::new();
    let mut rx = bus.subscribe();
    let w = silent(cache.clone(), bus);
    w.set(wanted(sb.0.to_str().unwrap(), &[], std::iter::empty()));
    changed(&mut rx, &sb.0, false).await;
    let listing = || cache.page(&sb.0, 0, 500, "", true, crate::listing_sort::Sort::default(), None);
    assert_eq!(serde_json::to_value(listing().await).unwrap()["total"], 0);
    std::fs::write(&path, "").unwrap();
    changed(&mut rx, &sb.0, false).await;
    assert_eq!(serde_json::to_value(listing().await).unwrap()["total"], 1);
    std::fs::remove_file(&path).unwrap();
    changed(&mut rx, &sb.0, false).await;
    assert_eq!(serde_json::to_value(listing().await).unwrap()["total"], 0);
}

#[tokio::test]
async fn an_edit_before_the_tab_workspace_save_is_announced_on_first_sample() {
    let sb = Sandbox::new("watch-registration");
    let path = sb.0.join("app.py");
    std::fs::write(&path, "initial").unwrap();
    let old = crate::files::etag(&std::fs::metadata(&path).unwrap());
    let bus = Bus::new();
    let mut rx = bus.subscribe();
    let w = silent(Cache::new(1 << 20), bus);
    // The browser read the file, then edited it externally before its tab was registered.
    std::fs::write(&path, "external edit before registration").unwrap();
    w.set(wanted(sb.0.to_str().unwrap(), &[], [path.display().to_string()].into_iter()));
    assert_ne!(changed(&mut rx, &path, true).await.unwrap(), old);
}
