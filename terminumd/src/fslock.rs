// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Checked directory locking (AC-99, AC-196), shared with the std-only checks log sink.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

// Darwin and Linux's stable open(2) flags; keep this module independent of Cargo dependencies.
#[cfg(target_os = "macos")]
pub const NOFOLLOW: i32 = 0x100;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const NOFOLLOW: i32 = 0x20000;
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const NOFOLLOW: i32 = 0x8000;
#[cfg(target_os = "macos")]
pub const DIRECTORY: i32 = 0x100000;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub const DIRECTORY: i32 = 0x10000;
#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
pub const DIRECTORY: i32 = 0x4000;

unsafe extern "C" { fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int; }

/// Does not create or follow a directory link; File drop releases the advisory lock.
pub fn directory(dir: &Path) -> io::Result<File> {
    let file = OpenOptions::new().read(true).custom_flags(DIRECTORY | NOFOLLOW).open(dir)?;
    loop {
        // SAFETY: the descriptor is our opened directory; LOCK_EX is 2 on both platforms.
        if unsafe { flock(file.as_raw_fd(), 2) } == 0 { return Ok(file) }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted { return Err(error) }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn ac196_flags_match_the_platform_headers() {
        assert_eq!(super::NOFOLLOW, libc::O_NOFOLLOW);
        assert_eq!(super::DIRECTORY, libc::O_DIRECTORY);
        assert_eq!(2, libc::LOCK_EX);
    }
}
