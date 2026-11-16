// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-197 retry evidence crosses the stage/tree boundary as quoted check output, never instructions.
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

/// Each integration stage starts without the prior candidate/round's note; standalone landing still adds its
/// smoke retries to its targeted ones. unlinkat removes a link itself, never its target.
pub(super) fn clear(tree: &Path) -> Result<(), String> {
    let clear = || -> io::Result<()> {
        let Some(dir) = directory(tree)? else { return Ok(()) };
        // SAFETY: dir owns an open directory fd and the fixed name is NUL-terminated.
        if unsafe { libc::unlinkat(dir.as_raw_fd(), c"retried".as_ptr(), 0) } != 0 {
            let e = io::Error::last_os_error();
            if e.kind() != io::ErrorKind::NotFound { return Err(e) }
        }
        Ok(())
    };
    clear().map_err(|e| format!("Retry diagnostics could not be cleared: {e}"))
}

fn read_file(tree: &Path) -> io::Result<Option<String>> {
    let Some(dir) = directory(tree)? else { return Ok(None) };
    // SAFETY: as above; NONBLOCK avoids waiting on a FIFO before metadata rejects non-regular files.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), c"retried".as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK) };
    if fd < 0 {
        let e = io::Error::last_os_error();
        return if e.kind() == io::ErrorKind::NotFound { Ok(None) } else { Err(e) };
    }
    // SAFETY: openat returned a new owned fd, transferred once to File.
    let file = unsafe { File::from_raw_fd(fd) };
    if !file.metadata()?.is_file() { return Err(io::Error::other(".checks/retried is not a regular file")) }
    let mut bytes = Vec::new();
    file.take((MOST + 1) as u64).read_to_end(&mut bytes)?;
    let truncated = bytes.len() > MOST;
    bytes.truncate(MOST);
    let text = crate::plans::rules::clean(&String::from_utf8_lossy(&bytes));
    // Redaction may expand short values; keep the quoted comment well below COMMENT_MOST too.
    let bounded: String = text.chars().take(MOST / 4).collect();
    if bounded.is_empty() { return Ok(None) }
    let mut quoted = bounded.lines().map(|line| format!("> {line}\n")).collect::<String>();
    if truncated || bounded.len() < text.len() { quoted.push_str("> [retry diagnostics truncated]\n"); }
    Ok(Some(quoted))
}

/// A missing/empty note needs no record; an unreadable one is said where its evidence would have been.
pub(super) fn read(tree: &Path) -> Option<String> {
    match read_file(tree) {
        Ok(note) => note,
        Err(e) => Some(format!("Retry diagnostics could not be read: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn ac197_notes_are_bounded_cleaned_redacted_and_cleared() {
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
        clear(&tree).unwrap();
        assert!(read(&tree).is_none());
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
