// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;

fn pending_scan(c: &Cache, path: &Path) -> (watch::Sender<Option<ReadResult>>, Pending, Arc<AtomicBool>) {
    let (tx, rx) = watch::channel(None);
    let pending = Pending { rx, fallback: None };
    let dirty = Arc::new(AtomicBool::new(false));
    c.slots.lock().insert(path.to_path_buf(), (Slot::Loading(pending.clone(), dirty.clone()), Instant::now()));
    (tx, pending, dirty)
}

#[tokio::test]
async fn invalidation_during_a_scan_keeps_waiters_until_fresh_publication() {
    let c = Cache::new(64 << 20);
    let path = PathBuf::from("/synthetic-folder");
    let (tx, first, dirty) = pending_scan(&c, &path);
    let second = c.start_read(path.clone());
    let mut rereads_left = DIRTY_REREADS;
    c.invalidate(&path);
    assert!(!c.finish_read(&path, &dirty, &tx, Err("obsolete scan".into()), &mut rereads_left));
    assert!(first.rx.borrow().is_none());
    assert!(matches!(c.slots.lock().get(&path), Some((Slot::Loading(..), _))));
    assert_eq!(c.generation.load(AO::Relaxed), 1, "no duplicate worker started");
    assert!(c.finish_read(&path, &dirty, &tx, Err("fresh scan".into()), &mut rereads_left));
    assert_eq!(second.wait().await.unwrap().err().unwrap(), "fresh scan");
    assert_eq!(first.wait().await.unwrap().err().unwrap(), "fresh scan");
}

#[tokio::test]
async fn continuously_invalidated_scans_publish_for_all_waiters_and_reread() {
    // AC-61: a change during each pass must not keep Files waiting for a quiet pass.
    let dir = crate::testutil::fresh_dir("changing-listing");
    fs::write(dir.join("first.txt"), "").unwrap();
    let c = Cache::new(64 << 20);
    let (tx, first, dirty) = pending_scan(&c, &dir);
    let second = c.start_read(dir.clone());
    let mut rereads_left = DIRTY_REREADS;
    c.invalidate(&dir);
    let initial = Arc::new(read_dir_sorted(&dir, 100).unwrap());
    assert!(!c.finish_read(&dir, &dirty, &tx, Ok(initial), &mut rereads_left));
    assert!(first.rx.borrow().is_none());

    c.invalidate(&dir);
    let last = Arc::new(read_dir_sorted(&dir, 101).unwrap());
    assert!(c.finish_read(&dir, &dirty, &tx, Ok(last.clone()), &mut rereads_left));
    for waiter in [first, second] {
        assert!(Arc::ptr_eq(&waiter.wait().await.unwrap().unwrap(), &last));
    }

    fs::write(dir.join("second.txt"), "").unwrap();
    let page = c.page(&dir, 0, 10, "", true, Sort::Name, None).await;
    assert_eq!((page.status, page.total), ("ready", 2));
    assert_ne!(page.generation, last.generation);
}

#[tokio::test(start_paused = true)]
async fn a_result_completed_between_polls_is_available_if_the_next_read_times_out() {
    // AC-61: the previous request timed out, leaving only the slot's receiver.
    let dir = crate::testutil::fresh_dir("listing-between-polls");
    let c = Cache::new(64 << 20);
    let (tx, _, dirty) = pending_scan(&c, &dir);
    let last = Arc::new(read_dir_sorted(&dir, 100).unwrap());
    c.invalidate(&dir);
    let mut rereads_left = 0;
    assert!(c.finish_read(&dir, &dirty, &tx, Ok(last.clone()), &mut rereads_left));
    c.invalidate(&dir);

    let mut next = c.start_read(dir);
    // Hold the new read open to exercise the wait limit independently of disk speed.
    let (_unfinished, rx) = watch::channel(None);
    next.rx = rx;
    assert!(Arc::ptr_eq(&next.wait().await.unwrap().unwrap(), &last));
}

#[test]
fn lossy_names_are_refused_instead_of_aliasing_another_file() {
    use std::os::unix::ffi::OsStringExt;
    assert!(entry_name(std::ffi::OsString::from_vec(b"a\xff.txt".to_vec())).is_err());
    assert_eq!(&*entry_name("a\u{fffd}.txt".into()).unwrap(), "a\u{fffd}.txt");
}

#[test]
fn errors_and_empty_folders_are_evicted_with_their_slot_overhead() {
    let c = Cache::new(1);
    c.slots.lock().insert(PathBuf::from("/missing"), (Slot::Done(Err("missing".into()), Some(Instant::now())), Instant::now()));
    let empty = Listing { entries: vec![], dir_mtime: 0, generation: 1, read_at: Instant::now(), bytes: 0, filtered: Mutex::new(None), filtered_bytes: Default::default(), recent: OnceLock::new(), by_ext: OnceLock::new() };
    c.slots.lock().insert(PathBuf::from("/empty"), (Slot::Done(Ok(Arc::new(empty)), Some(Instant::now())), Instant::now()));
    assert!(c.stats().0 > 1);
    c.evict();
    assert_eq!(c.stats(), (0, 0));
}

#[tokio::test]
async fn sort_and_filter_indexes_count_and_evict() {
    let dir = crate::testutil::temp_root().join(format!("tm-lsidx-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    for i in 0..2000 {
        fs::write(dir.join(format!("f{i}.txt")), b"").unwrap();
    }
    let c = Cache::new(usize::MAX);
    assert_eq!(c.page(&dir, 0, 10, "", true, Sort::Name, None).await.status, "ready");
    let plain = c.stats().0;
    c.page(&dir, 0, 10, "", true, Sort::Ext, None).await;
    let sorted = c.stats().0;
    assert!(sorted >= plain + 2000 * 4, "the Ext index is counted: {plain} -> {sorted}");
    c.page(&dir, 0, 10, "f1", true, Sort::Ext, None).await;
    assert!(c.stats().0 > sorted, "the filtered view is counted");
    // a limit that holds the folder but not its indexes: building one evicts it
    let small = Cache::new(plain + 1000);
    small.page(&dir, 0, 10, "", true, Sort::Name, None).await;
    assert_eq!(small.stats().1, 1, "the folder alone fits");
    let p = small.page(&dir, 0, 10, "", true, Sort::Recent, None).await;
    assert_eq!(p.status, "ready", "the reader still gets its page");
    assert_eq!(small.stats(), (0, 0), "the index pushed the cache over its limit");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn natural_order() {
    let mut v = vec!["f10.txt", "F2.txt", "f1.txt", "a", "B", "f02b", "Ärger", "z"];
    v.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(v, vec!["a", "B", "f1.txt", "F2.txt", "f02b", "f10.txt", "z", "Ärger"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_and_filter() {
    let dir = crate::testutil::temp_root().join(format!("fbd-test-{}", std::process::id()));
    fs::create_dir_all(dir.join("sub")).unwrap();
    for i in 0..1200 {
        fs::write(dir.join(format!("f{i}.txt")), b"x").unwrap();
    }
    fs::write(dir.join(".hidden"), b"").unwrap();
    let c = Cache::new(64 << 20);
    let p = c.page(&dir, 0, 3, "", true, Sort::Name, Some("f10.txt")).await;
    assert_eq!(p.total, 1202);
    assert_eq!(p.entries[0].n, "sub");
    assert_eq!(p.entries[1].n, ".hidden");
    assert_eq!(p.entries[2].n, "f0.txt");
    assert_eq!(p.entries[2].s, Some(1));
    assert_eq!(p.located, Some(2 + 10)); // sub, .hidden, f0..f9, then f10
    let beyond = c.page(&dir, usize::MAX, 500, "", true, Sort::Name, None).await;
    assert_eq!(beyond.status, "ready");
    assert!(beyond.entries.is_empty());
    let p = c.page(&dir, 0, 500, "f119", false, Sort::Name, None).await;
    assert_eq!(p.total, 11); // f119, f1190..f1199
    let p = c.page(&dir.join("missing"), 0, 10, "", true, Sort::Name, None).await;
    assert_eq!(p.status, "error");
    fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn pages_filter_and_locate_follow_the_order() {
    let dir = crate::testutil::temp_root().join(format!("tm-ls-sort-{}", std::process::id()));
    fs::create_dir_all(dir.join("sub")).unwrap();
    for n in ["b.txt", "a.md", "c.rs", "d.txt", ".env"] { fs::write(dir.join(n), b"x").unwrap(); }
    let c = Cache::new(64 << 20);
    let names = |p: &Page| p.entries.iter().map(|r| r.n.clone()).collect::<Vec<_>>();
    let p = c.page(&dir, 0, 10, "", true, Sort::Ext, Some("b.txt")).await;
    assert_eq!(names(&p), ["sub", ".env", "a.md", "c.rs", "b.txt", "d.txt"]);
    assert_eq!(p.located, Some(4));
    let p = c.page(&dir, 1, 2, "", false, Sort::Ext, Some("c.rs")).await;
    assert_eq!((names(&p), p.total, p.located), (vec!["a.md".into(), "c.rs".into()], 5, Some(2)), "hidden left out, a page of the order");
    let p = c.page(&dir, 0, 10, "t", true, Sort::Ext, None).await;
    assert_eq!(names(&p), ["b.txt", "d.txt"], "filtered in the order");
    let p = c.page(&dir, 0, 10, "t", true, Sort::Name, None).await;
    assert_eq!(names(&p), ["b.txt", "d.txt"]);
    assert_eq!(names(&c.page(&dir, 0, 10, "", true, Sort::Name, None).await), ["sub", ".env", "a.md", "b.txt", "c.rs", "d.txt"]);
    fs::remove_dir_all(&dir).unwrap();
}
