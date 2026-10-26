// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Unit-test helpers: a private tmux server (never the user's) that goes away with the test,
//! also when an assertion fails.

use std::path::PathBuf;
use std::process::{Command, Output};

/// Whether the tmux on PATH is at least `want` (major, minor): 3.4 types `send-keys S-Enter` as
/// text and writes `$` in a window name as `\$`; 3.8 keeps a window name's `\` as `\\`.
pub fn tmux_at_least(want: (u32, u32)) -> bool {
    let v = Command::new("tmux").arg("-V").output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let n: Vec<u32> = v.split(|c: char| !c.is_ascii_digit()).filter_map(|p| p.parse().ok()).take(2).collect();
    n.len() == 2 && (n[0], n[1]) >= want
}

pub struct Tmux {
    /// `-L` name: `tm-test-<name>-<pid>`
    pub socket: String,
    /// a scratch folder of its own, removed with it
    pub dir: PathBuf,
}

impl Tmux {
    /// A server whose session "main" (`cols`×`rows`) runs `cmd`; None when tmux is missing.
    pub fn start(name: &str, (cols, rows): (u16, u16), cmd: &str) -> Option<Tmux> {
        if Command::new("tmux").arg("-V").output().is_err() {
            eprintln!("skipped: tmux is not on PATH");
            return None;
        }
        let socket = format!("tm-test-{name}-{}", std::process::id());
        let dir = std::env::temp_dir().join(&socket);
        std::fs::create_dir_all(&dir).unwrap();
        let s = Tmux { dir: std::fs::canonicalize(&dir).unwrap(), socket };
        let out = s.tmux(&["new-session", "-d", "-s", "main", "-x", &cols.to_string(), "-y", &rows.to_string(), cmd]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        Some(s)
    }

    pub fn tmux(&self, args: &[&str]) -> Output {
        Command::new("tmux").args(["-u", "-L", &self.socket, "-f", "/dev/null"]).args(args).output().expect("tmux runs")
    }

    /// The output of a command that must succeed.
    pub fn read(&self, args: &[&str]) -> String {
        let out = self.tmux(args);
        assert!(out.status.success(), "{args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    /// Runs `lines` through tmux's command parser, as the control client sends them.
    pub fn source(&self, lines: &[String]) {
        let file = self.dir.join("cmds.conf");
        std::fs::write(&file, lines.join("\n") + "\n").unwrap();
        self.read(&["source-file", file.to_str().unwrap()]);
    }
}

/// An empty folder `tm-<name>-<pid>` in the temporary folder for one test.
pub fn fresh_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("tm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

impl Drop for Tmux {
    fn drop(&mut self) {
        let _ = self.tmux(&["kill-server"]);
        // where tmux puts `-L` sockets (also when a test killed the server itself)
        let base = std::env::var("TMUX_TMPDIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/tmp".into());
        // SAFETY: getuid has no preconditions
        let _ = std::fs::remove_file(PathBuf::from(base).join(format!("tmux-{}", unsafe { libc::getuid() })).join(&self.socket));
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}
