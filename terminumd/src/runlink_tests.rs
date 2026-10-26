// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;

use super::*;

/// A fresh short folder (socket paths stay under 104 bytes), removed on drop.
struct Tmp(PathBuf);
impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn tmp(tag: &str) -> Tmp {
    let p = std::env::temp_dir().join(format!("tmrl-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    Tmp(p)
}

#[test]
fn the_folder_is_made_private_and_a_foreign_one_refused() {
    let t = tmp("dir");
    let run = t.0.join("run");
    assert!(check_dir(&run, false).is_err(), "missing and not made");
    check_dir(&run, true).unwrap();
    assert_eq!(std::fs::metadata(&run).unwrap().permissions().mode() & 0o777, 0o700);
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(check_dir(&run, true).unwrap_err().contains("not 700"), "another's (open) folder");
    let real = t.0.join("real");
    std::fs::create_dir(&real).unwrap();
    std::fs::set_permissions(&real, std::fs::Permissions::from_mode(0o700)).unwrap();
    let ln = t.0.join("ln");
    std::os::unix::fs::symlink(&real, &ln).unwrap();
    assert!(check_dir(&ln, true).unwrap_err().contains("symlink"), "a symlinked folder");
    let s = t.0.join("ln/m-a-1.sock");
    let _l = UnixListener::bind(&s).unwrap();
    assert!(setup(&ln, "a", &s).is_err(), "never linked through a symlinked folder");
    assert!(links(&ln).is_empty());
    std::fs::write(t.0.join("file"), b"").unwrap();
    assert!(check_dir(&t.0.join("file"), true).unwrap_err().contains("not a folder"));
}

#[test]
fn the_link_points_at_the_socket_atomically_and_goes_with_it() {
    let t = tmp("link");
    let run = t.0.join("run");
    check_dir(&run, true).unwrap();
    let first = run.join("m-mac1-aaa.sock");
    let _l1 = UnixListener::bind(&first).unwrap();
    setup(&run, "mac1", &first).unwrap();
    assert_eq!(std::fs::read_link(link_path(&run, "mac1")).unwrap(), first);
    let second = run.join("m-mac1-bbb.sock");
    let _l2 = UnixListener::bind(&second).unwrap();
    setup(&run, "mac1", &second).unwrap();
    assert_eq!(std::fs::read_link(link_path(&run, "mac1")).unwrap(), second, "replaced, never missing");
    assert!(first.exists(), "a live socket of the same Mac is kept");
    assert!(!std::fs::read_dir(&run).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".tmp")), "no temp link left");
    unlink(&run, "mac1", &first);
    assert!(!first.exists());
    assert_eq!(std::fs::read_link(link_path(&run, "mac1")).unwrap(), second, "the newer connection's link stays");
    unlink(&run, "mac1", &second);
    assert!(std::fs::symlink_metadata(link_path(&run, "mac1")).is_err(), "its own link goes");
}

#[test]
fn only_this_macs_dead_sockets_are_removed() {
    let t = tmp("sweep");
    let run = t.0.join("run");
    check_dir(&run, true).unwrap();
    let dead = run.join("m-mac1-old.sock");
    drop(UnixListener::bind(&dead).unwrap());          // bound then closed: refuses
    let other_dead = run.join("m-mac2-old.sock");
    drop(UnixListener::bind(&other_dead).unwrap());
    let other_live = run.join("m-mac2-new.sock");
    let _l = UnixListener::bind(&other_live).unwrap();
    let mine = run.join("m-mac1-new.sock");
    let _m = UnixListener::bind(&mine).unwrap();
    std::fs::write(run.join("m-mac1-file.sock"), b"").unwrap();
    setup(&run, "mac1", &mine).unwrap();
    assert!(!dead.exists(), "own dead socket removed");
    assert!(other_dead.exists() && other_live.exists(), "another Mac's sockets are never touched");
    assert!(run.join("m-mac1-file.sock").exists(), "not a socket: kept");
    assert!(mine.exists());
}

#[test]
fn a_socket_outside_the_folder_or_a_bad_mac_is_refused() {
    let t = tmp("refuse");
    let run = t.0.join("run");
    check_dir(&run, true).unwrap();
    let out = t.0.join("m-mac1-x.sock");
    let _l = UnixListener::bind(&out).unwrap();
    assert!(setup(&run, "mac1", &out).unwrap_err().contains("not in"));
    let f = run.join("m-mac1-f.sock");
    std::fs::write(&f, b"").unwrap();
    assert!(setup(&run, "mac1", &f).unwrap_err().contains("not a socket"));
    assert!(setup(&run, "../x", &f).is_err());
    assert!(!mac_ok("") && !mac_ok("a/b") && mac_ok("Mac-1_x"));
}

#[test]
fn links_resolve_in_name_order_and_skip_broken_ones() {
    let t = tmp("links");
    let run = t.0.join("run");
    check_dir(&run, true).unwrap();
    let (a, b) = (run.join("m-b-1.sock"), run.join("m-a-1.sock"));
    let (_la, _lb) = (UnixListener::bind(&a).unwrap(), UnixListener::bind(&b).unwrap());
    link(&run, "b", &a).unwrap();
    link(&run, "a", &b).unwrap();
    std::os::unix::fs::symlink(t.0.join("elsewhere.sock"), run.join("mac-c.sock")).unwrap();
    assert_eq!(links(&run), vec![b, a], "mac-a before mac-b; a link out of the folder skipped");
}
