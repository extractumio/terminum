// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! launchd socket activation (AC-08): launchd listens on 127.0.0.1:<port> from the moment the
//! agent is loaded and hands the socket to each terminumd it starts, so no other program can
//! bind the page's port while terminumd is down, restarting or upgraded. Not run by launchd
//! (TM_NO_LAUNCHD=1, tests, Linux), terminumd binds the port itself (sock::bind_port).

/// What launchd gave for the `Listeners` socket of the plist.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug)]
pub enum Activation {
    /// not started by launchd (or no launchd): bind the port as before
    NotLaunchd,
    /// launchd's listening socket, bound to 127.0.0.1:<port>
    Held(std::net::TcpListener),
    /// launchd's socket is not bound to 127.0.0.1 (another program held the port when the
    /// agent was bootstrapped: launchd then hands an unbound socket and never tries again)
    Unbound,
}

/// What the message for `Activation::Unbound` says; the command looks for it in the log.
pub const UNBOUND: &str = "was held by another program when launchd loaded terminum";

/// The message terminumd logs and `terminum status` shows for `Activation::Unbound`.
pub fn unbound_message(port: u16) -> String {
    format!("port {port} {UNBOUND}: free it, then run `terminum restart`")
}

/// Judges launchd's socket: one bound to 127.0.0.1 and a port is ours.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn judge(l: std::net::TcpListener) -> Activation {
    match l.local_addr() {
        Ok(std::net::SocketAddr::V4(a)) if *a.ip() == std::net::Ipv4Addr::LOCALHOST && a.port() != 0 => Activation::Held(l),
        _ => Activation::Unbound,
    }
}

#[cfg(target_os = "macos")]
mod launchd {
    use std::os::fd::FromRawFd;
    use std::os::raw::{c_char, c_int};

    unsafe extern "C" {
        /// libSystem (launch.h): the fds launchd holds for socket `name` of the caller's job.
        fn launch_activate_socket(name: *const c_char, fds: *mut *mut c_int, cnt: *mut usize) -> c_int;
    }

    /// launchd's `Listeners` socket; Err(message) for anything but "not run by launchd".
    pub fn activate() -> Result<Option<std::net::TcpListener>, String> {
        let mut fds: *mut c_int = std::ptr::null_mut();
        let mut cnt: usize = 0;
        // SAFETY: a NUL-terminated name and two out-pointers launchd fills; the array it
        // returns is malloc'ed and freed below
        let r = unsafe { launch_activate_socket(c"Listeners".as_ptr(), &mut fds, &mut cnt) };
        if r == libc::ESRCH {
            return Ok(None);
        }
        if r != 0 {
            return Err(format!("launchd gave no socket: {}", std::io::Error::from_raw_os_error(r)));
        }
        // SAFETY: on success `fds` holds `cnt` descriptors that are now ours
        let got: Vec<c_int> = unsafe { std::slice::from_raw_parts(fds, cnt).to_vec() };
        // SAFETY: the array was allocated by launch_activate_socket with malloc
        unsafe { libc::free(fds.cast()) };
        let [fd] = got[..] else {
            for fd in got {
                // SAFETY: closing descriptors we own and do not use
                unsafe { libc::close(fd) };
            }
            return Err(format!("launchd gave {cnt} sockets for Listeners, expected 1"));
        };
        // SAFETY: fcntl on a descriptor we own; launchd hands it blocking (tokio needs
        // O_NONBLOCK) and with CLOEXEC, which is set again so tmux and ssh never inherit it
        unsafe {
            let fl = libc::fcntl(fd, libc::F_GETFL);
            if fl < 0 || libc::fcntl(fd, libc::F_SETFL, fl | libc::O_NONBLOCK) < 0 || libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) < 0 {
                let e = std::io::Error::last_os_error();
                libc::close(fd);
                return Err(format!("launchd's socket: {e}"));
            }
            Ok(Some(std::net::TcpListener::from_raw_fd(fd)))
        }
    }
}

/// launchd's socket for the page's port, judged; exits loudly when launchd failed.
pub fn listener() -> Activation {
    #[cfg(target_os = "macos")]
    match launchd::activate() {
        Ok(None) => Activation::NotLaunchd,
        Ok(Some(l)) => judge(l),
        Err(msg) => {
            eprintln!("error: {msg}");
            tracing::error!(event = "start.failed", msg);
            std::process::exit(2);
        }
    }
    #[cfg(not(target_os = "macos"))]
    Activation::NotLaunchd
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_socket_on_loopback_with_a_port_is_held() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        assert!(matches!(judge(l), Activation::Held(_)));
    }

    #[test]
    fn an_unbound_or_foreign_socket_is_not_ours() {
        // what launchd hands when the port was taken at bootstrap: a socket on 0.0.0.0:0
        let l = std::net::TcpListener::bind("0.0.0.0:0").unwrap();
        assert!(matches!(judge(l), Activation::Unbound));
        assert!(unbound_message(47900).contains("free it, then run `terminum restart`"));
    }

    #[test]
    fn outside_launchd_there_is_nothing_to_activate() {
        // cargo test is not a launchd job with Sockets: ESRCH, not an error
        assert!(matches!(listener(), Activation::NotLaunchd));
    }
}
