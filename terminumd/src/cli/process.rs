// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Bounded CLI subprocesses: own their process group and read pipes without detached threads.

use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

fn nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd belongs to a live pipe; fcntl changes only that descriptor's flags.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn read_some<R: Read>(pipe: &mut Option<R>, bytes: &mut Vec<u8>) -> io::Result<()> {
    let Some(reader) = pipe else { return Ok(()) };
    let mut buffer = [0; 4096];
    // A continuously writing child must still let the deadline and the other pipe run.
    for _ in 0..16 {
        match reader.read(&mut buffer) {
            Ok(0) => { *pipe = None; return Ok(()) }
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn collect(child: &mut Child, limit: Duration) -> Result<Output, String> {
    let (mut out, mut err) = (child.stdout.take(), child.stderr.take());
    nonblocking(out.as_ref().expect("piped").as_raw_fd()).map_err(|e| e.to_string())?;
    nonblocking(err.as_ref().expect("piped").as_raw_fd()).map_err(|e| e.to_string())?;
    let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
    let deadline = Instant::now() + limit;
    loop {
        read_some(&mut out, &mut stdout).map_err(|e| e.to_string())?;
        read_some(&mut err, &mut stderr).map_err(|e| e.to_string())?;
        // Keep the leader unreaped while a descendant holds either pipe: its pid/group
        // cannot be reused before timeout cleanup signals the group we created.
        if out.is_none() && err.is_none()
            && let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            return Ok(Output { status, stdout, stderr });
        }
        if Instant::now() >= deadline {
            return Err(format!("no answer within {} s", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// Capture both pipes and status within `limit`; errors close readers and end our group.
pub fn run_timeout(mut cmd: Command, limit: Duration) -> Result<Output, String> {
    let mut child = cmd.process_group(0).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| e.to_string())?;
    let result = collect(&mut child, limit);
    if result.is_err() {
        // SAFETY: process_group(0) made this group for the child we started, never a caller's.
        unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
        let _ = child.wait();
    }
    result
}
