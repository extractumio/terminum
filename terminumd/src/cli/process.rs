// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Bounded CLI subprocesses: own their session and process group, end with a terminal CLI or
//! nonterminal host helper (AC-99), and read pipes without detached threads.

use std::io::{self, Read};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
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
        if INTERRUPTED.load(Ordering::SeqCst) != 0 {
            return Err("interrupted".into());
        }
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

/// Cancellation is handled in normal code: a signal during spawn must not leave the new
/// session behind, and a signal after reap must never target a reused process group.
static INTERRUPTED: AtomicI32 = AtomicI32::new(0);
/// One `run_timeout` at a time: the handlers and INTERRUPTED are the process's own.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());
const PASSED_ON: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

extern "C" fn pass_on(sig: libc::c_int) {
    INTERRUPTED.store(sig, Ordering::SeqCst);
}

/// Terminal CLIs and host helpers own cancellation; daemon callers retain graceful shutdown.
/// The caller's handlers come back after the child is collected or ended.
fn pass_signals(host: bool) -> Vec<(libc::c_int, libc::sigaction)> {
    // SAFETY: isatty only reads the descriptor's state.
    if !host && unsafe { libc::isatty(0) } != 1 {
        return Vec::new();
    }
    PASSED_ON.iter().filter_map(|&sig| {
        // SAFETY: sigaction with zeroed structs is the documented way to read and set a handler.
        unsafe {
            let mut new: libc::sigaction = std::mem::zeroed();
            new.sa_sigaction = pass_on as *const () as libc::sighandler_t;
            libc::sigemptyset(&mut new.sa_mask);
            let mut old: libc::sigaction = std::mem::zeroed();
            libc::sigaction(sig, std::ptr::null(), &mut old);
            if old.sa_sigaction == libc::SIG_IGN {
                return None; // ignored (nohup): stays ignored
            }
            (libc::sigaction(sig, &new, std::ptr::null_mut()) == 0).then_some((sig, old))
        }
    }).collect()
}

fn restore_signals(old: Vec<(libc::c_int, libc::sigaction)>) {
    for (sig, act) in old {
        // SAFETY: `act` is the handler sigaction read for `sig`.
        unsafe { libc::sigaction(sig, &act, std::ptr::null_mut()) };
    }
    // Read after restoring: a signal between the collector's last poll and restoration must
    // still end the CLI; a later one is handled by the caller's restored disposition.
    let sig = INTERRUPTED.swap(0, Ordering::SeqCst);
    if sig != 0 {
        // SAFETY: restore the existing CLI cancellation semantics, after killing/reaping ours.
        unsafe { libc::signal(sig, libc::SIG_DFL); libc::raise(sig); }
    }
}

/// Capture both pipes and status within `limit`; errors close readers and end our group. The
/// child has a session of its own (no controlling terminal): a read of /dev/tty fails at once
/// instead of stopping it in the background until the limit.
pub fn run_timeout(cmd: Command, limit: Duration) -> Result<Output, String> {
    run(cmd, limit, false)
}

/// A host helper has no terminal: WATCH's TERM still ends its detached plugin call (AC-99).
pub fn run_host_timeout(cmd: Command, limit: Duration) -> Result<Output, String> {
    run(cmd, limit, true)
}

fn run(mut cmd: Command, limit: Duration, host: bool) -> Result<Output, String> {
    // SAFETY: setsid is async-signal-safe, as pre_exec requires.
    let cmd = unsafe { cmd.pre_exec(|| if libc::setsid() == -1 { Err(io::Error::last_os_error()) } else { Ok(()) }) };
    let _one = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let passed = pass_signals(host);
    let mut child = match cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn() {
        Ok(c) => c,
        Err(e) => { restore_signals(passed); return Err(e.to_string()); }
    };
    let result = collect(&mut child, limit);
    if result.is_err() {
        // SAFETY: setsid made this session's group for the child we started, never a caller's.
        unsafe { libc::killpg(child.id() as libc::pid_t, libc::SIGKILL) };
        let _ = child.wait();
    }
    restore_signals(passed);
    result
}
