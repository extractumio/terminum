// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Read retained history and follow inode replacements (AC-196), including startup failures.

use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use super::{Policy, lock_dir, segment};

#[derive(Clone, Copy, Default)]
pub struct Mark {
    dev: u64,
    ino: u64,
    offset: u64,
}

pub fn mark(path: &Path) -> Mark {
    std::fs::metadata(path).map(|m| Mark { dev: m.dev(), ino: m.ino(), offset: m.len() }).unwrap_or_default()
}

fn same(a: Mark, b: Mark) -> bool { (a.dev, a.ino) == (b.dev, b.ino) }

/// Take both file handles and their lengths while writers cannot rename or append.
fn files(path: &Path, policy: Policy) -> io::Result<Vec<(File, Mark)>> {
    let _dir = lock_dir(path.parent().unwrap_or(Path::new(".")))?;
    let mut opened = Vec::new();
    for n in (0..=policy.keep).rev() {
        let name = if n == 0 { path.to_path_buf() } else { segment(path, n) };
        match OpenOptions::new().read(true).custom_flags(crate::fslock::NOFOLLOW).open(name) {
            Ok(f) => {
                let m = f.metadata()?;
                opened.push((f, Mark { dev: m.dev(), ino: m.ino(), offset: m.len() }));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {},
            Err(e) => return Err(e),
        }
    }
    if opened.is_empty() { return Err(io::Error::new(io::ErrorKind::NotFound, "no log yet")) }
    Ok(opened)
}

/// Read backwards in small blocks: `logs -n 100` does not scan all retained megabytes.
fn tail(opened: Vec<(File, Mark)>, lines: usize, since: Option<Mark>) -> io::Result<Vec<u8>> {
    if lines == 0 { return Ok(Vec::new()) }
    let first = since.and_then(|m| opened.iter().position(|(_, at)| same(*at, m)));
    let mut chunks = VecDeque::new();
    let mut breaks = 0;
    for (i, (mut f, at)) in opened.into_iter().enumerate().rev() {
        if first.is_some_and(|first| i < first) { break }
        let start = if first == Some(i) { since.map_or(0, |m| m.offset.min(at.offset)) } else { 0 };
        let mut pos = at.offset;
        while pos > start && breaks <= lines {
            let n = (pos - start).min(8192) as usize;
            pos -= n as u64;
            f.seek(SeekFrom::Start(pos))?;
            let mut chunk = vec![0; n];
            f.read_exact(&mut chunk)?;
            breaks += chunk.iter().filter(|b| **b == b'\n').count();
            chunks.push_front(chunk);
        }
        if breaks > lines { break }
    }
    let bytes: Vec<u8> = chunks.into_iter().flatten().collect();
    let mut breaks = 0;
    let mut start = 0;
    for (i, b) in bytes.iter().enumerate().rev() {
        if *b == b'\n' && i + 1 < bytes.len() {
            breaks += 1;
            if breaks == lines { start = i + 1; break }
        }
    }
    Ok(bytes[start..].to_vec())
}

/// Newest lines in chronological order; `since` spans a rename of the marked file.
pub fn recent(path: &Path, policy: Policy, lines: usize, since: Option<Mark>) -> io::Result<Vec<u8>> {
    tail(files(path, policy)?, lines, since)
}

/// Drain the marked inode and each newer segment, then remember the active offset.
fn next(path: &Path, policy: Policy, cursor: &mut Mark, out: &mut impl Write) -> io::Result<()> {
    let opened = files(path, policy)?;
    let first = opened.iter().position(|(_, at)| same(*at, *cursor)).unwrap_or(0);
    for (mut f, at) in opened.into_iter().skip(first) {
        let offset = if same(at, *cursor) { cursor.offset.min(at.offset) } else { 0 };
        f.seek(SeekFrom::Start(offset))?;
        io::copy(&mut Read::by_ref(&mut f).take(at.offset - offset), out)?;
        *cursor = at;
    }
    out.flush()
}

pub fn follow(path: &Path, policy: Policy, lines: usize) -> io::Result<()> {
    let opened = files(path, policy)?;
    let mut cursor = opened.last().expect("a nonempty snapshot").1;
    let text = tail(opened, lines, None)?;
    let mut out = io::stdout().lock();
    out.write_all(&text)?;
    out.flush()?;
    loop {
        next(path, policy, &mut cursor, &mut out)?;
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

#[cfg(test)]
pub(super) fn read_next(path: &Path, policy: Policy, cursor: &mut Mark) -> Vec<u8> {
    let mut out = Vec::new();
    next(path, policy, cursor, &mut out).unwrap();
    out
}
