// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Regression cases for conditional saves and exclusive rename.

use super::*;
use std::sync::{Arc, Barrier};

fn setup(tag: &str) -> (Roots, PathBuf) {
    let dir = crate::testutil::fresh_dir(&format!("regression-{tag}"));
    (Roots::parse(&dir.display().to_string()), dir)
}

#[test]
fn competing_saves_conflict_even_through_symlink_aliases() {
    let (roots, dir) = setup("save");
    let roots = Arc::new(roots);
    let file = dir.join("file.txt");
    fs::write(&file, "original").unwrap();
    let alias = dir.join("alias.txt");
    std::os::unix::fs::symlink(&file, &alias).unwrap();
    let original = etag(&fs::metadata(&file).unwrap());
    let start = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8).map(|n| {
        let (roots, start, original) = (roots.clone(), start.clone(), original.clone());
        let path = if n % 2 == 0 { file.clone() } else { alias.clone() };
        std::thread::spawn(move || {
            let text = format!("writer {n}\n").repeat(16_384);
            start.wait();
            (save(&roots, &path, &text, &original), text)
        })
    }).collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    let winners: Vec<_> = results.iter().filter(|(r, _)| r.is_ok()).collect();
    assert_eq!(winners.len(), 1);
    assert_eq!(fs::read_to_string(&file).unwrap(), winners[0].1);
    assert_eq!(etag(&fs::metadata(&file).unwrap()), *winners[0].0.as_ref().unwrap());
    assert!(results.iter().filter(|(r, _)| r.is_err()).all(|(r, _)| matches!(r, Err(OpError::Conflict { .. }))));
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn maximum_length_names_save_without_long_temporary_names() {
    let (roots, dir) = setup("long");
    for name in ["x".repeat(255), format!("{}x", "é".repeat(127))] {
        let file = touch(&roots, &dir, &name).unwrap();
        save(&roots, &file, "saved", "*").unwrap();
        assert_eq!(fs::read_to_string(file).unwrap(), "saved");
    }
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_save_racing_rename_never_recreates_the_old_path() {
    let (roots, dir) = setup("save-rename");
    let roots = Arc::new(roots);
    let file = dir.join("old.txt");
    let updated = "new content\n".repeat(1 << 20);
    for _ in 0..8 {
        fs::write(&file, "original").unwrap();
        let original = etag(&fs::metadata(&file).unwrap());
        let (start, copy, r, path) = (Arc::new(Barrier::new(2)), updated.clone(), roots.clone(), file.clone());
        let other = start.clone();
        let saving = std::thread::spawn(move || { other.wait(); save(&r, &path, &copy, &original) });
        start.wait();
        let target = rename(&roots, &file, "new.txt").unwrap();
        let saved = saving.join().unwrap();
        assert!(!file.exists(), "Save must not resurrect the renamed path");
        let contents = fs::read_to_string(&target).unwrap();
        if saved.is_ok() { assert_eq!(contents, updated); }
        else { assert!(matches!(saved, Err(OpError::Conflict { .. }))); assert_eq!(contents, "original"); }
        fs::remove_file(target).unwrap();
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn exclusive_rename_preserves_a_target_created_after_preflight() {
    let (_, dir) = setup("rename");
    let (from, to) = (dir.join("from"), dir.join("to"));
    fs::write(&from, "source").unwrap();
    assert!(!to.exists()); // the endpoint's preflight
    fs::write(&to, "other writer").unwrap();
    assert_eq!(rename_exclusive(&from, &to).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read_to_string(&from).unwrap(), "source");
    assert_eq!(fs::read_to_string(&to).unwrap(), "other writer");
    fs::remove_file(&to).unwrap();
    fs::hard_link(&from, &to).unwrap();
    assert_eq!(rename_exclusive(&from, &to).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    fs::remove_file(&to).unwrap();
    fs::create_dir(&to).unwrap();
    assert_eq!(rename_exclusive(&from, &to).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    fs::remove_dir(&to).unwrap();
    std::os::unix::fs::symlink(&from, &to).unwrap();
    assert_eq!(rename_exclusive(&from, &to).unwrap_err().kind(), std::io::ErrorKind::AlreadyExists);
    assert!(to.symlink_metadata().unwrap().file_type().is_symlink());
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn an_upload_is_a_new_file_never_written_over_another() {
    let (roots, dir) = setup("upload");
    let p = upload(&roots, &dir, "notes.txt", b"one", false).unwrap();
    assert_eq!((p.clone(), fs::read(&p).unwrap()), (dir.join("notes.txt"), b"one".to_vec()));
    // taken: refused, the file as it was; with both: the next free number
    assert_eq!(upload(&roots, &dir, "notes.txt", b"two", false).unwrap_err().code(), "exists");
    assert_eq!(fs::read(&p).unwrap(), b"one");
    assert_eq!(upload(&roots, &dir, "notes.txt", b"two", true).unwrap(), dir.join("notes 2.txt"));
    assert_eq!(upload(&roots, &dir, "notes.txt", b"three", true).unwrap(), dir.join("notes 3.txt"));
    assert_eq!(fs::read(dir.join("notes 3.txt")).unwrap(), b"three");
    // a symlink in the name's place is taken, never followed
    std::os::unix::fs::symlink(dir.join("elsewhere"), dir.join("link")).unwrap();
    assert_eq!(upload(&roots, &dir, "link", b"x", false).unwrap_err().code(), "exists");
    assert!(!dir.join("elsewhere").exists());
    // a name taken by a link to a file outside the roots: taken, and Keep Both saves beside it
    let outside = crate::testutil::fresh_dir("regression-upload-outside");
    fs::write(outside.join("far.txt"), "far").unwrap();
    std::os::unix::fs::symlink(outside.join("far.txt"), dir.join("far.txt")).unwrap();
    assert_eq!(upload(&roots, &dir, "far.txt", b"x", false).unwrap_err().code(), "exists");
    assert_eq!(upload(&roots, &dir, "far.txt", b"near", true).unwrap(), dir.join("far 2.txt"));
    assert_eq!((fs::read(outside.join("far.txt")).unwrap(), fs::read_link(dir.join("far.txt")).unwrap()), (b"far".to_vec(), outside.join("far.txt")));
    // no part files left behind
    assert!(fs::read_dir(&dir).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".part")));
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(outside).unwrap();
}

#[test]
fn an_upload_keeps_to_the_roots_and_plain_names() {
    let (roots, dir) = setup("upload-roots");
    assert_eq!(upload(&roots, Path::new("/tmp"), "x", b"x", false).unwrap_err().code(), "outside_writable_roots");
    // a folder in the roots that is a link to one outside: outside
    let outside = crate::testutil::fresh_dir("regression-upload-roots-outside");
    std::os::unix::fs::symlink(&outside, dir.join("out")).unwrap();
    assert_eq!(upload(&roots, &dir.join("out"), "x", b"x", false).unwrap_err().code(), "outside_writable_roots");
    assert!(!outside.join("x").exists());
    fs::remove_dir_all(&outside).unwrap();
    for bad in ["", "..", "a/b", "a\0b"] {
        assert_eq!(upload(&roots, &dir, bad, b"x", false).unwrap_err().code(), "bad_name", "{bad:?}");
    }
    assert_eq!(upload(&roots, &dir.join("gone"), "x", b"x", false).unwrap_err().code(), "not_found");
    assert_eq!(numbered(".bashrc", 2), ".bashrc 2");
    assert_eq!(numbered("a.tar.gz", 2), "a.tar 2.gz");
    assert_eq!(numbered("README", 4), "README 4");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_new_file_or_folder_on_a_name_a_link_outside_took_is_a_name_taken() {
    let (roots, dir) = setup("touch-link");
    let outside = crate::testutil::fresh_dir("regression-touch-outside");
    std::os::unix::fs::symlink(outside.join("far.txt"), dir.join("far.txt")).unwrap();   // dangling: never created there
    std::os::unix::fs::symlink(&outside, dir.join("far")).unwrap();
    assert_eq!(touch(&roots, &dir, "far.txt").unwrap_err().code(), "exists");
    assert_eq!(mkdir(&roots, &dir, "far").unwrap_err().code(), "exists");
    assert!(!outside.join("far.txt").exists(), "nothing written outside");
    // a folder that is a link outside: outside, for both
    assert_eq!(touch(&roots, &dir.join("far"), "x").unwrap_err().code(), "outside_writable_roots");
    assert_eq!(mkdir(&roots, &dir.join("far"), "x").unwrap_err().code(), "outside_writable_roots");
    assert!(!outside.join("x").exists());
    fs::remove_dir_all(dir).unwrap();
    fs::remove_dir_all(outside).unwrap();
}
