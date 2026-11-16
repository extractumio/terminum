// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Completed integration logs (AC-196): bounded snapshots and incremental retention.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs::{self, ReadDir};
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::{INTEGRATION, TURN, create_dir, lock_dir, remove};

const SCAN: usize = 500;
const DELETES: usize = 16;

/// A completed step's bounded tail; new captured output, never a copy of an old log.
pub fn snapshot(path: &Path, text: &str) -> io::Result<()> {
    let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
    create_dir(path.parent().unwrap_or(Path::new(".")))?;
    let _dir = lock_dir(path.parent().unwrap_or(Path::new(".")))?;
    let mut start = text.len().saturating_sub(INTEGRATION.bytes as usize);
    while !text.is_char_boundary(start) { start += 1; }
    crate::config::write_private(path, &text.as_bytes()[start..])
}

struct Project {
    dir: PathBuf,
    entries: ReadDir,
    newest: BinaryHeap<Reverse<(SystemTime, PathBuf)>>,
}

/// Keeps an iterator and only the newest K paths, never a list/sort of all legacy files.
pub struct Sweeper {
    root: PathBuf,
    projects: Option<ReadDir>,
    current: Option<Project>,
}

impl Sweeper {
    pub fn new(home: PathBuf) -> Self {
        Self { root: home.join("integration"), projects: None, current: None }
    }

    /// One maintenance minute's fixed budget; the next call resumes exactly where it stopped.
    pub fn tick(&mut self) -> io::Result<()> {
        if self.projects.is_none() {
            match fs::read_dir(&self.root) {
                Ok(dirs) => self.projects = Some(dirs),
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e),
            }
        }
        let mut deletes = 0;
        for _ in 0..SCAN {
            if deletes >= DELETES { break }
            let Some(project) = self.current.as_mut() else {
                match self.projects.as_mut().and_then(Iterator::next) {
                    Some(entry) => {
                        let entry = entry?;
                        if entry.file_type()?.is_dir() {
                            self.current = Some(Project { dir: entry.path(), entries: fs::read_dir(entry.path())?, newest: BinaryHeap::new() });
                        }
                    }
                    None => { self.projects = None; break }
                }
                continue;
            };
            let _turn = TURN.lock().unwrap_or_else(|e| e.into_inner());
            let _dir = match lock_dir(&project.dir) {
                Ok(dir) => dir,
                Err(e) if e.kind() == io::ErrorKind::NotFound => { self.current = None; continue },
                Err(e) => return Err(e),
            };
            let Some(entry) = project.entries.next() else { self.current = None; continue };
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() || !is_step(&path) { continue }
            let modified = entry.metadata()?.modified()?;
            if SystemTime::now().duration_since(modified).is_ok_and(|age| age >= INTEGRATION.age) {
                remove(&path)?;
                deletes += 1;
                continue;
            }
            project.newest.push(Reverse((modified, path)));
            if project.newest.len() > INTEGRATION.keep {
                let Reverse((_, old)) = project.newest.pop().expect("more than K files");
                remove(&old)?;
                deletes += 1;
            }
        }
        Ok(())
    }
}

fn is_step(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n|
        (n.starts_with("batch-") || n.starts_with("verify-")) && n.ends_with(".log"))
}
