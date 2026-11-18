// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-197 retry evidence, and AC-146's timed checks skipped for a busy machine, cross the stage/tree boundary as
//! quoted check output, never instructions; a red Verify's failed steps (AC-198) as names alone, checked here.
use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

const MOST: usize = 64 << 10;

/// Open the fixed diagnostics directory without following a link. Holding it makes the final open/unlink safe
/// even if a command replaces .checks meanwhile.
fn directory(tree: &Path) -> io::Result<Option<File>> {
    match OpenOptions::new().read(true).custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW).open(tree.join(".checks")) {
        Ok(dir) => Ok(Some(dir)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The notes a check leaves in `.checks`, by file name, with what they are called where said.
const RETRIED: (&CStr, &str) = (c"retried", "Retry diagnostics");
const SKIPPED: (&CStr, &str) = (c"skipped", "Skipped checks");
const FAILED: &CStr = c"failed";
/// The most failed steps a note may name (§9 Limits).
const STEPS_MOST: usize = 16;

/// Each integration stage starts without the prior candidate/round's notes; standalone landing still adds its
/// smoke retries to its targeted ones. unlinkat removes a link itself, never its target.
pub(super) fn clear(tree: &Path) -> Result<(), String> {
    let clear = || -> io::Result<()> {
        let Some(dir) = directory(tree)? else { return Ok(()) };
        for name in [RETRIED.0, SKIPPED.0, FAILED] {
            // SAFETY: dir owns an open directory fd and the fixed name is NUL-terminated.
            if unsafe { libc::unlinkat(dir.as_raw_fd(), name.as_ptr(), 0) } != 0 {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::NotFound { return Err(e) }
            }
        }
        Ok(())
    };
    clear().map_err(|e| format!("Retry diagnostics, skipped checks or failed steps could not be cleared: {e}"))
}

/// A note's bytes, at most MOST + 1 of them, from a regular file reached without following a link.
fn raw(tree: &Path, name: &CStr) -> io::Result<Option<Vec<u8>>> {
    let Some(dir) = directory(tree)? else { return Ok(None) };
    // SAFETY: as above; NONBLOCK avoids waiting on a FIFO before metadata rejects non-regular files.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), name.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK) };
    if fd < 0 {
        let e = io::Error::last_os_error();
        return if e.kind() == io::ErrorKind::NotFound { Ok(None) } else { Err(e) };
    }
    // SAFETY: openat returned a new owned fd, transferred once to File.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() { return Err(io::Error::other(format!(".checks/{} is not a regular file", name.to_string_lossy()))) }
    let mut bytes = Vec::new();
    file.take((MOST + 1) as u64).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

fn read_file(tree: &Path, (name, what): (&CStr, &str)) -> io::Result<Option<String>> {
    let Some(mut bytes) = raw(tree, name)? else { return Ok(None) };
    let truncated = bytes.len() > MOST;
    bytes.truncate(MOST);
    let text = crate::plans::rules::clean(&String::from_utf8_lossy(&bytes));
    // Redaction may expand short values; keep the quoted comment well below COMMENT_MOST too.
    let bounded: String = text.chars().take(MOST / 4).collect();
    if bounded.is_empty() { return Ok(None) }
    let mut quoted = bounded.lines().map(|line| format!("> {line}\n")).collect::<String>();
    if truncated || bounded.len() < text.len() { quoted.push_str(&format!("> [{} truncated]\n", what.to_lowercase())); }
    Ok(Some(quoted))
}

/// A missing/empty note needs no record; an unreadable one is said where its evidence would have been.
fn note(tree: &Path, kind: (&CStr, &str)) -> Option<String> {
    match read_file(tree, kind) {
        Ok(note) => note,
        Err(e) => Some(format!("{} could not be read: {e}", kind.1)),
    }
}

/// The checks that passed only on their retry (AC-197).
pub(super) fn read(tree: &Path) -> Option<String> {
    note(tree, RETRIED)
}

/// The timed checks a busy machine skipped (AC-146): a pass without them.
pub(super) fn skipped(tree: &Path) -> Option<String> {
    note(tree, SKIPPED)
}

/// The steps a red Verify names as failed (AC-198): each a word of letters, digits, `_`, `.` or `-`, at most
/// STEPS_MOST, in order, once each. A note that is missing, unreadable, empty or holds any other line names none:
/// the task is then judged as without a note, never on part of a list.
pub(super) fn failed(tree: &Path) -> Option<Vec<String>> {
    let text = String::from_utf8(raw(tree, FAILED).ok()??).ok()?;
    let mut steps: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let word = line.len() <= 64 && line.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b));
        if !word { return None }
        if !steps.iter().any(|s| s == line) { steps.push(line.to_string()) }
    }
    (!steps.is_empty() && steps.len() <= STEPS_MOST).then_some(steps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn ac197_ac146_notes_are_bounded_cleaned_redacted_and_cleared() {
        let tree = crate::testutil::fresh_dir("retry-note");
        assert!(read(&tree).is_none());
        clear(&tree).unwrap();
        let checks = tree.join(".checks");
        std::fs::create_dir(&checks).unwrap();
        std::fs::write(checks.join("retried"), format!("critical\nPASSWORD=fixture-value\u{7}\u{202e}\n{}", "x".repeat(MOST + 100))).unwrap();
        let note = read(&tree).unwrap();
        assert!(note.starts_with("> critical\n> PASSWORD=[redacted]\n"), "{note}");
        assert!(!note.contains("fixture-value") && !note.contains('\u{7}') && !note.contains('\u{202e}'));
        assert!(note.ends_with("> [retry diagnostics truncated]\n") && note.len() < MOST);
        std::fs::write(checks.join("skipped"), "targeted: SKIPPED timed checks, the machine busy for 15 minutes: critical\n").unwrap();
        assert_eq!(skipped(&tree).as_deref(), Some("> targeted: SKIPPED timed checks, the machine busy for 15 minutes: critical\n"));
        clear(&tree).unwrap();
        assert!(read(&tree).is_none() && skipped(&tree).is_none(), "each stage starts without either note");
    }

    #[test]
    fn ac198_failed_steps_are_names_alone_or_none() {
        let tree = crate::testutil::fresh_dir("failed-note");
        assert_eq!(failed(&tree), None, "no note");
        let checks = tree.join(".checks");
        std::fs::create_dir(&checks).unwrap();
        let with = |text: &str| { std::fs::write(checks.join("failed"), text).unwrap(); failed(&tree) };
        assert_eq!(with("critical\ncargo\n\ncritical\ne2e_core\nui-unit\npage.x\n"), Some(vec!["critical".into(), "cargo".into(), "e2e_core".into(), "ui-unit".into(), "page.x".into()]));
        for bad in ["critical\nrm -rf ~\n", "$(id)\n", "a;b\n", "\n", "", "kritisch\u{e9}\n", &"x".repeat(65)] {
            assert_eq!(with(bad), None, "{bad:?}: any other line names none");
        }
        assert_eq!(with(&(0..17).map(|i| format!("s{i}\n")).collect::<String>()), None, "over the most");
        assert_eq!(with(&(0..16).map(|i| format!("s{i}\n")).collect::<String>()).map(|s| s.len()), Some(16));
        clear(&tree).unwrap();
        assert_eq!(failed(&tree), None, "cleared with the other notes");
        std::fs::write(tree.join("other"), "critical\n").unwrap();
        symlink(tree.join("other"), checks.join("failed")).unwrap();
        assert_eq!(failed(&tree), None, "a link is not followed");
    }

    #[test]
    fn ac197_notes_never_follow_a_link_or_read_a_nonregular_file() {
        let tree = crate::testutil::fresh_dir("retry-link");
        let checks = tree.join(".checks");
        std::fs::create_dir(&checks).unwrap();
        let other = tree.join("other");
        std::fs::write(&other, "unrelated fixture").unwrap();
        symlink(&other, checks.join("retried")).unwrap();
        assert!(read(&tree).unwrap().contains("could not be read"));
        clear(&tree).unwrap();
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "unrelated fixture");
        std::fs::create_dir(checks.join("retried")).unwrap();
        assert!(read(&tree).unwrap().contains("not a regular file"));
        assert!(clear(&tree).is_err());
        std::fs::remove_dir_all(&checks).unwrap();
        symlink(&tree, &checks).unwrap();
        assert!(read(&tree).unwrap().contains("could not be read"));
        assert!(clear(&tree).is_err());
    }
}
