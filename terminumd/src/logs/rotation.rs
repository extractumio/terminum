// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Shared rotation core (AC-196), also compiled into the checks' standalone log sink.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DAY: Duration = Duration::from_secs(86_400);
pub const EVERY: Duration = Duration::from_secs(60);
const MIB: u64 = 1 << 20;
pub(super) static TURN: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[derive(Clone, Copy)]
pub struct Policy {
    pub bytes: u64,
    pub period: Duration,
    pub keep: usize,
    pub age: Duration,
}

pub const DAEMON: Policy = Policy { bytes: 10 * MIB, period: DAY, keep: 7, age: Duration::from_secs(7 * 86_400) };
pub const AGENT: Policy = Policy { bytes: MIB, period: DAY, keep: 3, age: Duration::from_secs(3 * 86_400) };
#[allow(dead_code)] // the standalone checks sink compiles this same module
pub const CHECKS: Policy = Policy { bytes: 10 * MIB, period: DAY, keep: 2, age: Duration::from_secs(7 * 86_400) };
pub const INTEGRATION: Policy = Policy { bytes: 2 * MIB, period: Duration::from_secs(30 * 86_400), keep: 100, age: Duration::from_secs(30 * 86_400) };

pub(super) fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

pub(super) fn lock_dir(dir: &Path) -> io::Result<File> {
    fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    crate::fslock::directory(dir)
}

fn private_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).mode(0o600).custom_flags(crate::fslock::NOFOLLOW).open(path)
}

pub fn segment(path: &Path, n: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{n}"));
    PathBuf::from(name)
}

fn since_file(path: &Path) -> PathBuf {
    path.with_file_name(format!(".{}.since", path.file_name().unwrap_or_default().to_string_lossy()))
}

pub(super) fn set_since(path: &Path, at: u64) -> io::Result<()> {
    // Only eight bytes of clock data; a unique 0600 temporary file, atomically renamed.
    let target = since_file(path);
    let temp = target.with_extension(format!("since.{}.{}.new", std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).mode(0o600)
            .custom_flags(crate::fslock::NOFOLLOW).open(&temp)?;
        file.write_all(at.to_string().as_bytes())?;
        file.sync_all()?;
        fs::rename(&temp, &target)
    })();
    if result.is_err() { let _ = fs::remove_file(&temp); }
    result
}

fn since(path: &Path, at: u64) -> io::Result<u64> {
    let state = since_file(path);
    if let Ok(mut file) = OpenOptions::new().read(true).custom_flags(crate::fslock::NOFOLLOW).open(state) {
        let mut text = String::new();
        Read::by_ref(&mut file).take(32).read_to_string(&mut text)?;
        if let Ok(n) = text.parse::<u64>() { return Ok(n.min(at)) }
    }
    let started = fs::metadata(path).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(at, |d| d.as_secs());
    set_since(path, started)?;
    Ok(started)
}

pub(super) fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn rename(from: &Path, to: &Path) -> io::Result<()> {
    match fs::rename(from, to) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn rotate(path: &Path, policy: Policy, at: u64) -> io::Result<()> {
    remove(&segment(path, policy.keep))?;
    for n in (1..policy.keep).rev() { rename(&segment(path, n), &segment(path, n + 1))?; }
    rename(path, &segment(path, 1))?;
    // Persist the new clock before opening: a crash cannot give a fresh segment an old clock.
    set_since(path, at)?;
    private_file(path)?;
    Ok(())
}

fn expire(path: &Path, policy: Policy, at: u64) -> io::Result<()> {
    for n in 1..=policy.keep {
        let old = segment(path, n);
        let modified = fs::metadata(&old).and_then(|m| m.modified()).ok()
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok()).map(|m| m.as_secs());
        if modified.is_some_and(|m| at.saturating_sub(m) >= policy.age.as_secs()) { remove(&old)?; }
    }
    Ok(())
}

/// A rotating stream. No cached append fd: a second process may rotate between batches.
pub struct Log {
    path: PathBuf,
    policy: Policy,
    checked: Instant,
    failed: Option<Instant>,
}

impl Log {
    pub fn open(path: PathBuf, policy: Policy) -> io::Result<Self> {
        let mut log = Self { path, policy, checked: Instant::now() - EVERY, failed: None };
        log.maintain()?;
        Ok(log)
    }

    fn may_rotate(&self) -> bool { self.failed.is_none_or(|at| at.elapsed() >= EVERY) }

    fn rotation_failed(&mut self, error: io::Error) -> io::Result<()> {
        self.failed = Some(Instant::now());
        let mut file = private_file(&self.path)?;
        writeln!(file, "ERROR event=log.rotation msg=\"Rotation failed; retry in 60 s\" error={error}")
    }

    fn rotate(&mut self, at: u64) -> io::Result<()> {
        match rotate(&self.path, self.policy, at) {
            Ok(()) => { self.failed = None; Ok(()) }
            Err(e) => self.rotation_failed(e),
        }
    }

    pub fn maintain(&mut self) -> io::Result<()> {
        self.append(&[])
    }

    pub fn append(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
        let _dir = lock_dir(self.path.parent().unwrap_or(Path::new(".")))?;
        let at = now();
        if self.failed.is_some_and(|at| at.elapsed() >= EVERY) { self.failed = None; }
        let timed = self.checked.elapsed() >= EVERY;
        if timed && self.may_rotate() {
            self.checked = Instant::now();
            let started = since(&self.path, at)?;
            let size = fs::metadata(&self.path).map_or(0, |m| m.len());
            if size > 0 && (size >= self.policy.bytes || at.saturating_sub(started) >= self.policy.period.as_secs()) {
                self.rotate(at)?;
            }
            if let Err(e) = expire(&self.path, self.policy, at) { self.rotation_failed(e)?; }
        }
        let mut file = private_file(&self.path)?;
        while !bytes.is_empty() {
            let size = file.metadata()?.len();
            if self.may_rotate() && (size >= self.policy.bytes || (size > 0 && size + bytes.len() as u64 > self.policy.bytes)) {
                drop(file);
                self.rotate(at)?;
                file = private_file(&self.path)?;
            }
            let take = if self.failed.is_some() { bytes.len() } else { bytes.len().min(self.policy.bytes as usize) };
            file.write_all(&bytes[..take])?;
            bytes = &bytes[take..];
        }
        Ok(())
    }
}

/// Adopt the old daemon name as an archive without loading either file into memory.
pub fn adopt_legacy(path: &Path) -> io::Result<()> {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    let _dir = lock_dir(path.parent().unwrap_or(Path::new(".")))?;
    let old = path.with_file_name("terminusd.log");
    if old.is_file() {
        // Keep current history newer than the adopted file, at the oldest retained slot.
        rename(&old, &segment(path, DAEMON.keep))?;
    }
    expire(path, DAEMON, now())
}

