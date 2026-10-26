// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A host's run folder `~/.terminum/run` (remote agents' progress): the Mac's ssh forwards its
//! hook socket there as `m-<macid>-<rand>.sock`, the host's helper points `mac-<macid>.sock`
//! at it, and `terminum-host hook|mcp` on that host post to every such link. The folder is
//! trusted only when it is ours: owned by this uid, mode 0700, not a symlink; a socket only
//! when it is ours and lies in it.

use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt};
use std::path::{Path, PathBuf};

fn uid() -> u32 {
    // SAFETY: geteuid has no preconditions
    unsafe { libc::geteuid() }
}

/// `~/.terminum/run` (`TM_HOME/run`), never created here.
pub fn dir() -> PathBuf {
    std::env::var_os("TM_HOME").map(PathBuf::from).unwrap_or_else(|| crate::config::home().join(".terminum")).join("run")
}

/// A Mac's id: 1–40 letters, digits, `-` or `_`.
pub fn mac_ok(mac: &str) -> bool {
    (1..=40).contains(&mac.len()) && mac.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The folder is ours (owner, 0700, a real folder); `create` makes it when missing.
pub fn check_dir(dir: &Path, create: bool) -> Result<(), String> {
    let at = dir.display();
    let meta = match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && create => {
            std::fs::DirBuilder::new().mode(0o700).create(dir).map_err(|e| format!("{at}: {e}"))?;
            std::fs::symlink_metadata(dir).map_err(|e| format!("{at}: {e}"))?
        }
        Err(e) => return Err(format!("{at}: {e}")),
        Ok(m) => m,
    };
    if meta.file_type().is_symlink() {
        return Err(format!("{at} is a symlink: refused"));
    }
    if !meta.is_dir() {
        return Err(format!("{at} is not a folder: refused"));
    }
    if meta.uid() != uid() {
        return Err(format!("{at} belongs to uid {}, not {}: refused", meta.uid(), uid()));
    }
    if meta.mode() & 0o777 != 0o700 {
        return Err(format!("{at} has mode {:o}, not 700: refused", meta.mode() & 0o777));
    }
    Ok(())
}

/// `sock` is a socket of this uid directly in `dir` (not through a symlink).
pub fn check_socket(dir: &Path, sock: &Path) -> Result<(), String> {
    let at = sock.display();
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    if sock.parent().and_then(canon).is_none() || sock.parent().and_then(canon) != canon(dir) {
        return Err(format!("{at} is not in {}: refused", dir.display()));
    }
    let meta = std::fs::symlink_metadata(sock).map_err(|e| format!("{at}: {e}"))?;
    if !meta.file_type().is_socket() {
        return Err(format!("{at} is not a socket: refused"));
    }
    if meta.uid() != uid() {
        return Err(format!("{at} belongs to uid {}: refused", meta.uid()));
    }
    Ok(())
}

pub fn link_path(dir: &Path, mac: &str) -> PathBuf {
    dir.join(format!("mac-{mac}.sock"))
}

/// Points `mac-<mac>.sock` at `sock` atomically (a temp symlink renamed over it).
pub fn link(dir: &Path, mac: &str, sock: &Path) -> Result<PathBuf, String> {
    let tmp = dir.join(format!(".mac-{mac}.{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(sock, &tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
    let to = link_path(dir, mac);
    std::fs::rename(&tmp, &to).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", to.display())
    })?;
    Ok(to)
}

/// A sweep's wait for one socket: a stuck listener (a full backlog) never holds the helper's start.
const SWEEP_WAIT: std::time::Duration = std::time::Duration::from_millis(200);

/// Connects to the socket at `path` within `wait`: std's connect has no timeout, and Linux
/// blocks a connect to a full backlog until the socket's send timeout (macOS refuses it at once).
pub fn connect_within(path: &Path, wait: std::time::Duration) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::os::fd::FromRawFd;
    use std::os::unix::ffi::OsStrExt;
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: sockaddr_un is plain data; all zeroes is a valid value
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.is_empty() || bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "not a socket path"));
    }
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (d, b) in addr.sun_path.iter_mut().zip(bytes) {
        *d = *b as libc::c_char;
    }
    // SAFETY: a plain socket(2); its fd is owned by the stream at once (closed on every error)
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a socket of ours, owned by nothing else
    let s = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
    // SAFETY: fcntl on our own fd
    unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) };
    s.set_write_timeout(Some(wait.max(std::time::Duration::from_millis(1))))?;   // SO_SNDTIMEO bounds the connect
    let len = (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len()) as libc::socklen_t;
    // SAFETY: addr is a valid sockaddr_un of `len` bytes
    if unsafe { libc::connect(fd, (&raw const addr).cast::<libc::sockaddr>(), len) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(s)
}

/// Removes this Mac's `m-<mac>-*.sock` other than `keep` that refuse a connection: a dead
/// ssh's leftovers. Never another Mac's, never by age, never one that answers or times out
/// (each connect waits at most SWEEP_WAIT).
pub fn sweep(dir: &Path, mac: &str, keep: &Path) -> Vec<PathBuf> {
    let prefix = format!("m-{mac}-");
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut gone = Vec::new();
    for e in rd.flatten() {
        let (p, name) = (e.path(), e.file_name().to_string_lossy().to_string());
        if !name.starts_with(&prefix) || !name.ends_with(".sock") || p == keep || check_socket(dir, &p).is_err() {
            continue;
        }
        let refused = connect_within(&p, SWEEP_WAIT).err().is_some_and(|e| e.kind() == std::io::ErrorKind::ConnectionRefused);
        if refused && std::fs::remove_file(&p).is_ok() {
            gone.push(p);
        }
    }
    gone
}

/// On exit: removes `sock`, and the Mac's link if it still points at it (a newer
/// connection of the same Mac may have taken it over).
pub fn unlink(dir: &Path, mac: &str, sock: &Path) {
    let _ = std::fs::remove_file(sock);
    let l = link_path(dir, mac);
    if std::fs::read_link(&l).is_ok_and(|t| t == sock) {
        let _ = std::fs::remove_file(&l);
    }
}

/// What `--link <sock> --mac <mac>` does before the helper says ready: the folder checked or
/// made, the socket checked, the link pointed at it, this Mac's dead sockets removed.
pub fn setup(dir: &Path, mac: &str, sock: &Path) -> Result<(), String> {
    if !mac_ok(mac) {
        return Err(format!("--mac {mac:?}: letters, digits, - and _ only"));
    }
    check_dir(dir, true)?;
    check_socket(dir, sock)?;
    link(dir, mac, sock)?;
    sweep(dir, mac, sock);
    Ok(())
}

/// The run folder's links (`mac-*.sock`) resolved to the sockets they point at, by name;
/// none when the folder is not ours, a link whose socket fails the checks is skipped.
pub fn links(dir: &Path) -> Vec<PathBuf> {
    if check_dir(dir, false).is_err() {
        return Vec::new();
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<String> = rd.flatten().map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("mac-") && n.ends_with(".sock")).collect();
    names.sort();
    names.into_iter().filter_map(|n| {
        let t = std::fs::read_link(dir.join(&n)).ok()?;
        let t = if t.is_absolute() { t } else { dir.join(t) };
        check_socket(dir, &t).ok().map(|_| t)
    }).collect()
}

#[cfg(test)]
#[path = "runlink_tests.rs"]
mod tests;
