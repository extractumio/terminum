// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-08, AC-16 against the real launchd, opt-in (TM_LAUNCHD_TEST=1; never in `make test`):
//! an agent of its own (io.terminum.test-<pid>, a free port, a private tmux socket, a
//! temporary root) loaded through `terminum restart`. Checks that launchd holds the port
//! across a kill and a kickstart (a squatter, SO_REUSEPORT or not, gets EADDRINUSE; a
//! connection made meanwhile is served after the restart), that `terminum restart` right
//! after that connection (its TIME_WAIT on the port) ends with launchd holding the port,
//! and that a port taken when the agent is loaded is reported, then taken back by
//! `terminum restart`. The agent is booted
//! out and the tmux server killed when the test ends, also on a failure.

#![cfg(target_os = "macos")]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{Duration, Instant};

struct Agent {
    dir: PathBuf,
    root: PathBuf,
    label: String,
    socket: String,
    port: u16,
}

impl Drop for Agent {
    fn drop(&mut self) {
        let uid = unsafe { libc::getuid() };
        let _ = Command::new("/bin/launchctl").arg("bootout").arg(format!("gui/{uid}/{}", self.label)).output();
        for tmux in ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux"] {
            let _ = Command::new(tmux).args(["-L", &self.socket, "kill-server"]).output();
        }
        let tmp = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".into());
        let _ = std::fs::remove_file(format!("{tmp}/tmux-{uid}/{}", self.socket));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Agent {
    fn terminum(&self, args: &[&str]) -> Output {
        let out = Command::new(self.dir.join("terminum"))
            .args(args)
            .env("HOME", &self.dir)
            .env("TM_HOME", &self.root)
            .env("TM_LAUNCH_AGENTS", self.dir.join("la"))
            .env("TM_LAUNCHD_LABEL", &self.label)
            .env("NO_COLOR", "1")
            .env_remove("TM_APP_DIR")
            .env_remove("TM_NO_LAUNCHD")
            .output()
            .unwrap();
        println!("$ terminum {}\n{}{}", args.join(" "), String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        out
    }

    /// terminumd's pid from /health on its socket, once it answers.
    fn pid(&self, within: Duration) -> Option<u64> {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if let Ok(mut s) = std::os::unix::net::UnixStream::connect(self.root.join("state/terminumd.sock")) {
                let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
                let _ = s.write_all(b"GET /health HTTP/1.0\r\nHost: localhost\r\n\r\n");
                let mut raw = String::new();
                let _ = s.read_to_string(&mut raw);
                if let Some(body) = raw.split("\r\n\r\n").nth(1)
                    && let Ok(v) = serde_json::from_str::<serde_json::Value>(body)
                {
                    assert_eq!(v["port"].as_u64(), Some(self.port as u64), "/health reports launchd's port");
                    return v["pid"].as_u64();
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        None
    }
}

/// bind(127.0.0.1:port) as another program would, with or without SO_REUSEPORT: the errno.
fn squat(port: u16, reuseport: bool) -> Option<i32> {
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
        let one: libc::c_int = 1;
        if reuseport {
            libc::setsockopt(fd, libc::SOL_SOCKET, libc::SO_REUSEPORT, (&raw const one).cast(), 4);
        }
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
        addr.sin_family = libc::AF_INET as u8;
        addr.sin_port = port.to_be();
        addr.sin_addr.s_addr = u32::from_be_bytes([127, 0, 0, 1]).to_be();
        let r = libc::bind(fd, (&raw const addr).cast(), std::mem::size_of::<libc::sockaddr_in>() as u32);
        let err = (r != 0).then(|| std::io::Error::last_os_error().raw_os_error().unwrap_or(0));
        libc::close(fd);
        err
    }
}

fn held(port: u16) -> bool {
    squat(port, false) == Some(libc::EADDRINUSE) && squat(port, true) == Some(libc::EADDRINUSE)
}

fn setup() -> Agent {
    let pid = std::process::id();
    let dir = PathBuf::from(format!("/tmp/tml-{pid}"));
    let _ = std::fs::remove_dir_all(&dir);
    let root = dir.join("r");
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let agent = Agent { dir: dir.clone(), root: root.clone(), label: format!("io.terminum.test-{pid}"), socket: format!("tm-test-launchd-{pid}"), port };
    let exe = Path::new(env!("CARGO_BIN_EXE_terminumd"));
    std::fs::create_dir_all(root.join("builds/t")).unwrap();
    std::fs::create_dir_all(root.join("state")).unwrap();
    std::os::unix::fs::symlink(exe, root.join("builds/t/terminumd")).unwrap();
    std::os::unix::fs::symlink("t", root.join("builds/current")).unwrap();
    std::os::unix::fs::symlink(exe, dir.join("terminum")).unwrap();
    std::fs::write(root.join("state/no-update-check"), "").unwrap();
    let settings = serde_json::json!({"port": port, "restore": false, "tmux": {"socket": agent.socket}});
    std::fs::write(root.join("settings.json"), settings.to_string()).unwrap();
    agent
}

#[test]
fn launchd_holds_the_port() {
    if std::env::var("TM_LAUNCHD_TEST").as_deref() != Ok("1") {
        eprintln!("skipped: the real launchd test runs with TM_LAUNCHD_TEST=1");
        return;
    }
    let a = setup();
    let out = a.terminum(&["restart"]);
    assert!(out.status.success(), "terminum restart");
    let first = a.pid(Duration::from_secs(10)).expect("terminumd answers");
    assert!(held(a.port), "launchd's socket refuses another program");

    // killed: launchd keeps the port; a connection made meanwhile is served after the restart
    unsafe { libc::kill(first as i32, libc::SIGKILL) };
    assert!(held(a.port), "held while terminumd is dead");
    let mut early = std::net::TcpStream::connect(("127.0.0.1", a.port)).expect("queued by launchd");
    early.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    write!(early, "GET /api/hello HTTP/1.0\r\nHost: 127.0.0.1:{}\r\n\r\n", a.port).unwrap();
    let mut answer = String::new();
    let _ = early.read_to_string(&mut answer);
    assert!(answer.starts_with("HTTP/1."), "the queued connection is answered: {answer:?}");
    drop(early);
    let second = a.pid(Duration::from_secs(10)).expect("launchd started terminumd again");
    assert_ne!(first, second);

    // kickstart -k, as upgrades and rollbacks restart it
    let uid = unsafe { libc::getuid() };
    let kick = Command::new("/bin/launchctl").args(["kickstart", "-k", &format!("gui/{uid}/{}", a.label)]).status().unwrap();
    assert!(kick.success());
    assert!(held(a.port), "held across a kickstart");
    let third = a.pid(Duration::from_secs(10)).expect("terminumd after the kickstart");
    assert_ne!(third, second);

    // restart at once: the connection above left a TIME_WAIT on the port, which launchd binds through
    let out = a.terminum(&["restart"]);
    assert!(out.status.success(), "terminum restart right after a served connection: {}", String::from_utf8_lossy(&out.stderr));
    assert!(a.pid(Duration::from_secs(10)).is_some());
    assert!(held(a.port));

    // a port taken when the agent is loaded: reported, then taken back with terminum restart
    let unload = Command::new("/bin/launchctl").arg("bootout").arg(format!("gui/{uid}/{}", a.label)).status().unwrap();
    assert!(unload.success());
    // the queued connection above left a TIME_WAIT on the port, which blocks a new listener
    // until it expires (2 MSL, 30 s on macOS)
    let deadline = Instant::now() + Duration::from_secs(60);
    let squatter = loop {
        let e = match std::net::TcpListener::bind(("127.0.0.1", a.port)) {
            Ok(l) => break l,
            Err(e) => e,
        };
        if Instant::now() >= deadline {
            let ns = Command::new("/usr/sbin/netstat").args(["-anv", "-p", "tcp"]).output().unwrap();
            let lines: Vec<String> = String::from_utf8_lossy(&ns.stdout).lines().filter(|l| l.contains(&a.port.to_string())).map(str::to_string).collect();
            panic!("the port is not free after bootout: {e}\n{}", lines.join("\n"));
        }
        std::thread::sleep(Duration::from_millis(250));
    };
    let out = a.terminum(&["restart"]);
    let said = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(!out.status.success());
    let reason = format!("port {} was held by another program when launchd loaded terminum: free it, then run `terminum restart`", a.port);
    assert!(said.contains(&reason), "restart says why: {said}");
    let status = String::from_utf8_lossy(&a.terminum(&["status"]).stdout).to_string();
    assert!(status.contains(&format!("not running — terminumd does not answer: {reason}")), "status says why: {status}");
    drop(squatter);
    let out = a.terminum(&["restart"]);
    assert!(out.status.success(), "terminum restart once the port is free");
    assert!(a.pid(Duration::from_secs(10)).is_some());
    assert!(held(a.port));
    println!("launchd test: PASS (label {}, port {})", a.label, a.port);
}
