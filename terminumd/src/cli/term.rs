// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A secret typed at the terminal without echo (the web password, AC-10): asked on /dev/tty
//! with ECHO off and the terminal restored after, also when reading fails or on ⌃C.

use std::io::{BufRead, Write};
use std::os::fd::AsRawFd;
use std::sync::OnceLock;

/// The terminal and its settings before the prompt, for the ⌃C handler.
static SAVED: OnceLock<(i32, libc::termios)> = OnceLock::new();

/// ⌃C at the prompt: echo back on, then the usual exit of an interrupted command.
extern "C" fn interrupted(_: libc::c_int) {
    if let Some((fd, t)) = SAVED.get() {
        // SAFETY: tcsetattr, write and _exit are async-signal-safe
        unsafe {
            libc::tcsetattr(*fd, libc::TCSAFLUSH, t);
            libc::write(*fd, b"\n".as_ptr().cast(), 1);
            libc::_exit(130);
        }
    }
}

pub fn read_secret(prompt: &str) -> Result<String, String> {
    let tty = std::fs::OpenOptions::new().read(true).write(true).open("/dev/tty").map_err(|e| format!("no terminal to ask for the password ({e}); set TM_WEB_PASSWORD"))?;
    let fd = tty.as_raw_fd();
    // SAFETY: termios is plain data filled by tcgetattr on our open terminal
    let mut saved: libc::termios = unsafe { std::mem::zeroed() };
    // SAFETY: tcgetattr writes into `saved`
    if unsafe { libc::tcgetattr(fd, &mut saved) } != 0 {
        return Err(format!("the terminal: {}", std::io::Error::last_os_error()));
    }
    let _ = SAVED.set((fd, saved));
    // SAFETY: installs a handler that only calls async-signal-safe functions
    let before = unsafe { libc::signal(libc::SIGINT, interrupted as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    let mut quiet = saved;
    quiet.c_lflag &= !libc::ECHO;
    quiet.c_lflag |= libc::ECHONL;
    (&tty).write_all(prompt.as_bytes()).and_then(|_| (&tty).flush()).map_err(|e| e.to_string())?;
    // SAFETY: tcsetattr with a termios derived from the terminal's own
    if unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &quiet) } != 0 {
        return Err(format!("the terminal: {}", std::io::Error::last_os_error()));
    }
    let mut line = String::new();
    let read = std::io::BufReader::new(&tty).read_line(&mut line);
    // SAFETY: restores the settings read above and the previous ⌃C handler
    unsafe {
        libc::tcsetattr(fd, libc::TCSAFLUSH, &saved);
        libc::signal(libc::SIGINT, before);
    }
    read.map_err(|e| e.to_string())?;
    Ok(line.trim_end_matches(['\n', '\r']).to_string())
}
