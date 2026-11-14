// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-196: size, age, concurrent processes, readers and writer drain.
use super::*;
use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use super::rotation::{now, set_since};
use std::process::Command;
use tracing_subscriber::fmt::MakeWriter;

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let path = crate::testutil::temp_root().join(format!("tm-logs-{}-{}", std::process::id(), rand::random::<u64>()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> PathBuf { self.0.join("test.log") }
}
impl Drop for Temp { fn drop(&mut self) { fs::remove_dir_all(&self.0).unwrap(); } }
fn policy(bytes: u64, keep: usize) -> Policy { Policy { bytes, keep, ..DAEMON } }

#[test]
fn ac196_size_rotates_writer_continues_and_only_k_archives_stay() {
    let dir = Temp::new();
    let path = dir.path();
    let p = policy(8, 2);
    let mut log = Log::open(path.clone(), p).unwrap();
    for line in [b"one-one\n", b"two-two\n", b"tri-tri\n", b"for-for\n"] { log.append(line).unwrap(); }
    assert_eq!(fs::read(&path).unwrap(), b"for-for\n");
    assert_eq!(fs::read(segment(&path, 1)).unwrap(), b"tri-tri\n");
    assert_eq!(fs::read(segment(&path, 2)).unwrap(), b"two-two\n");
    assert!(!segment(&path, 3).exists());
    assert_eq!(recent(&path, p, 2, None).unwrap(), b"tri-tri\nfor-for\n");
}

#[test]
fn ac196_under_limits_keeps_inode_and_bytes() {
    use std::os::unix::fs::MetadataExt;
    let dir = Temp::new();
    let path = dir.path();
    let mut log = Log::open(path.clone(), policy(1024, 2)).unwrap();
    log.append(b"first\n").unwrap();
    let inode = fs::metadata(&path).unwrap().ino();
    log.append(b"second\n").unwrap();
    log.maintain().unwrap();
    assert_eq!(fs::metadata(&path).unwrap().ino(), inode);
    assert_eq!(fs::read(&path).unwrap(), b"first\nsecond\n");
    assert!(!segment(&path, 1).exists());
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
}

#[test]
fn ac196_period_survives_restart_and_a_busy_files_mtime() {
    let dir = Temp::new();
    let path = dir.path();
    fs::write(&path, b"old\n").unwrap();
    set_since(&path, now() - 2 * 86_400).unwrap();
    let mut log = Log::open(path.clone(), DAEMON).unwrap();
    log.append(b"new\n").unwrap();
    assert_eq!(fs::read(segment(&path, 1)).unwrap(), b"old\n");
    assert_eq!(fs::read(path).unwrap(), b"new\n");
}

#[test]
fn ac196_expiry_removes_only_old_owned_files() {
    let dir = Temp::new();
    let path = dir.path();
    fs::write(segment(&path, 1), b"old\n").unwrap();
    fs::write(dir.0.join("other.log"), b"keep\n").unwrap();
    Log::open(path.clone(), Policy { age: Duration::ZERO, ..DAEMON }).unwrap();
    assert!(!segment(&path, 1).exists());
    assert!(dir.0.join("other.log").exists());
}

#[test]
fn ac196_oversized_record_is_segmented_without_loss() {
    let dir = Temp::new();
    let path = dir.path();
    let p = policy(8, 10);
    let mut log = Log::open(path.clone(), p).unwrap();
    let bytes = b"a long record with no missing bytes\n";
    log.append(bytes).unwrap();
    assert_eq!(recent(&path, p, 100, None).unwrap(), bytes);
    for n in 0..=p.keep {
        let path = if n == 0 { path.clone() } else { segment(&path, n) };
        if let Ok(m) = fs::metadata(path) { assert!(m.len() <= p.bytes); }
    }
}

#[test]
fn ac196_two_processes_rotate_without_losing_or_duplicating_lines() {
    let dir = Temp::new();
    let exe = std::env::current_exe().unwrap();
    let mut children = Vec::new();
    for writer in 0..2 {
        children.push(Command::new(&exe).args(["--exact", "logs::tests::ac196_child_writer", "--nocapture"])
            .env("TM_TEST_LOG_DIR", &dir.0).env("TM_TEST_LOG_WRITER", writer.to_string()).spawn().unwrap());
    }
    let codes: Vec<_> = children.iter_mut().map(|c| c.wait().unwrap()).collect();
    assert!(codes.iter().all(|s| s.success()), "{codes:?}");
    let text = String::from_utf8(recent(&dir.path(), policy(256, 100), 1000, None).unwrap()).unwrap();
    let mut lines: Vec<_> = text.lines().collect();
    lines.sort_unstable();
    let mut expected: Vec<_> = (0..2).flat_map(|w| (0..200).map(move |n| format!("{w}:{n:04}"))).collect();
    expected.sort_unstable();
    assert_eq!(lines, expected);
}

#[test]
fn ac196_child_writer() {
    let Some(dir) = std::env::var_os("TM_TEST_LOG_DIR") else { return };
    let writer = std::env::var("TM_TEST_LOG_WRITER").unwrap();
    let mut log = Log::open(PathBuf::from(dir).join("test.log"), policy(256, 100)).unwrap();
    for n in 0..200 { log.append(format!("{writer}:{n:04}\n").as_bytes()).unwrap(); }
}

#[test]
fn ac196_bounded_queue_keeps_all_records_before_worker_exit() {
    let dir = Temp::new();
    let path = dir.path();
    let p = policy(256, 100);
    let writer = writer::Writer::start(path.clone(), p, None).unwrap();
    std::thread::scope(|scope| {
        for w in 0..4 {
            let writer = &writer;
            scope.spawn(move || { for n in 0..100 { writeln!(writer.make_writer(), "{w}:{n:04}").unwrap(); } });
        }
    });
    drop(writer); // disconnect, drain, and join before reading or removing the folder
    let bytes = recent(&path, p, 1000, None).unwrap();
    assert_eq!(bytes.split(|b| *b == b'\n').filter(|b| !b.is_empty()).count(), 400);
}

#[test]
fn ac196_follow_and_startup_mark_span_replacement() {
    let dir = Temp::new();
    let path = dir.path();
    let p = policy(10, 3);
    let mut log = Log::open(path.clone(), p).unwrap();
    log.append(b"before\n").unwrap();
    let mut cursor = mark(&path);
    let start = cursor;
    log.append(b"error: new\n").unwrap();
    assert_eq!(reader::read_next(&path, p, &mut cursor), b"error: new\n");
    assert!(reader::read_next(&path, p, &mut cursor).is_empty());
    assert_eq!(recent(&path, p, 100, Some(start)).unwrap(), b"error: new\n");
}

#[test]
fn ac196_step_snapshots_are_bounded_and_expire_without_more_writes() {
    let dir = Temp::new();
    let project = dir.0.join("integration/test-project");
    fs::create_dir_all(&project).unwrap();
    for i in 0..105 { snapshot(&project.join(format!("batch-{i}.1.check.log")), "okay\n").unwrap(); }
    let mut sweep = Sweeper::new(dir.0.clone());
    sweep.tick().unwrap();
    assert_eq!(fs::read_dir(&project).unwrap().count(), 100);
    assert!(project.join("batch-104.1.check.log").exists());
    let path = project.join("batch-106.1.check.log");
    snapshot(&path, &"x".repeat(INTEGRATION.bytes as usize + 100)).unwrap();
    assert_eq!(fs::metadata(&path).unwrap().len(), INTEGRATION.bytes);
    let old = SystemTime::now() - INTEGRATION.age - Duration::from_secs(86_400);
    File::options().write(true).open(&path).unwrap().set_modified(old).unwrap();
    sweep.tick().unwrap();
    assert!(!path.exists());
}

#[test]
fn ac196_rotation_failure_keeps_existing_data_and_can_be_retried() {
    let dir = Temp::new();
    let path = dir.path();
    let p = policy(6, 1);
    let mut log = Log::open(path.clone(), p).unwrap();
    log.append(b"full\n").unwrap();
    fs::create_dir(segment(&path, 1)).unwrap();
    log.append(b"next\n").unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.starts_with("full\n") && text.ends_with("next\n") && text.contains("log.rotation"));
    fs::remove_dir(segment(&path, 1)).unwrap();
    drop(log);
    let mut log = Log::open(path.clone(), p).unwrap();
    log.append(b"after\n").unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"after\n");
}

#[test]
fn ac196_worker_keeps_records_when_rotation_fails_and_recovers_on_reopen() {
    let dir = Temp::new();
    let path = dir.path();
    let p = policy(64, 3);
    let writer = writer::Writer::start(path.clone(), p, None).unwrap();
    writeln!(writer.make_writer(), "{}", "a".repeat(63)).unwrap();
    writer.flush();
    fs::create_dir(segment(&path, 3)).unwrap();
    writeln!(writer.make_writer(), "kept during refusal").unwrap();
    writer.flush();
    assert!(fs::read_to_string(&path).unwrap().contains("kept during refusal"));
    assert!(fs::read_to_string(&path).unwrap().contains("log.rotation"));
    fs::remove_dir(segment(&path, 3)).unwrap();
    drop(writer);
    let writer = writer::Writer::start(path.clone(), p, None).unwrap();
    writeln!(writer.make_writer(), "after recovery").unwrap();
    writer.flush();
    let text = String::from_utf8(recent(&path, p, 100, None).unwrap()).unwrap();
    assert_eq!(text.matches("kept during refusal").count(), 1);
    assert!(text.ends_with("after recovery\n"));
}

#[test]
fn ac196_legacy_cleanup_has_a_fixed_delete_budget() {
    let dir = Temp::new();
    let project = dir.0.join("integration/test-project");
    fs::create_dir_all(&project).unwrap();
    let old = SystemTime::now() - INTEGRATION.age - Duration::from_secs(86_400);
    for i in 0..40 {
        let path = project.join(format!("batch-{i}.1.check.log"));
        fs::write(&path, "old\n").unwrap();
        File::options().write(true).open(path).unwrap().set_modified(old).unwrap();
    }
    let mut sweep = Sweeper::new(dir.0.clone());
    sweep.tick().unwrap();
    assert_eq!(fs::read_dir(&project).unwrap().count(), 24);
    sweep.tick().unwrap();
    assert_eq!(fs::read_dir(&project).unwrap().count(), 8);
    sweep.tick().unwrap();
    assert_eq!(fs::read_dir(&project).unwrap().count(), 0);
}

#[test]
fn ac196_exit_flush_works_after_rust_thread_locals_are_destroyed() {
    let dir = Temp::new();
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "logs::tests::ac196_exit_child"])
        .env("TM_TEST_EXIT_LOG", dir.path()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("panicked"));
    assert!(fs::read_to_string(dir.path()).unwrap().contains("kept at exit"));
}

#[test]
fn ac196_exit_child() {
    let Some(path) = std::env::var_os("TM_TEST_EXIT_LOG") else { return };
    let writer = writer::Writer::start(PathBuf::from(path), DAEMON, None).unwrap();
    writer::install(writer.clone());
    writeln!(writer.make_writer(), "kept at exit").unwrap();
    // Return normally: Rust tears down main's TLS before libc invokes atexit.
}
