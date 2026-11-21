// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Where a project's Check, Publish and After run (`terminum-integration.md` AC-190, §9 "Commands"): each line in a
//! window of terminumd's own tmux server, by the user's login shell with the batch's variables, marked
//! `@terminum-integration`; Dispatcher's window wrapper writes the line's status as `@terminum-exit` and keeps the
//! pane, so a check outlives a restart of terminumd and its status and output are read after. Nothing is typed in.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs a line and follows it; a test stands in for tmux.
pub trait Runner: Send + Sync {
    /// Starts `line` in `folder` with `vars`; its handle (a pane and its mark, `pane_of`).
    fn start(&self, label: &str, folder: &Path, line: &str, vars: &[(&str, String)]) -> Result<String, String>;
    /// Its status once it ended, None while it runs; Err when its window is gone.
    fn status(&self, handle: &str) -> Result<Option<i32>, String>;
    /// What it printed.
    fn output(&self, handle: &str) -> String;
    /// Ends what it runs: SIGTERM to its process group, SIGKILL 10 s later.
    fn stop(&self, handle: &str);
    /// Its window closed, the line ended.
    fn close(&self, handle: &str);
}

/// `export K='v'; …; line`: the variables before the human's line, which runs as typed.
pub fn with_vars(line: &str, vars: &[(&str, String)]) -> String {
    let mut out: String = vars.iter().map(|(k, v)| format!("export {k}={}; ", crate::termtext::posix_word(v))).collect();
    out.push_str(line);
    out
}

/// terminumd's local tmux server, reached as `tmux -L <socket> -f <conf>`.
pub struct Tmux {
    pub tmux: PathBuf,
    pub socket: String,
    pub conf: PathBuf,
}

/// The session the integration's windows stand in.
const SESSION: &str = "integration";

/// The pane of a handle `"%5 batch-7.1.check"`: the pane and the mark its line was started with. A server started
/// anew gives its pane IDs from %0 again, so the mark tells a lost window from another line's that took its ID; a
/// handle kept before marks were in it is the pane alone.
pub fn pane_of(handle: &str) -> &str {
    parts(handle).0
}

/// A handle's pane and the mark its line started with, none in a handle kept before marks were in it.
fn parts(handle: &str) -> (&str, Option<&str>) {
    handle.split_once(' ').map_or((handle, None), |(pane, label)| (pane, Some(label)))
}

impl Tmux {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let o = Command::new(&self.tmux).args(["-L", &self.socket, "-f"]).arg(&self.conf).args(args).output().map_err(|e| format!("tmux: {e}"))?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    /// What the handle's window is now: the line's own, with its status once it ended; gone (a pane of another's, or
    /// of another line after the server started anew, is never ours); or not known, tmux not answering for a moment.
    fn read<'a>(&self, handle: &'a str) -> Window<'a> {
        let (pane, label) = parts(handle);
        let said = match self.run(&["display", "-p", "-t", pane, "#{@terminum-integration}|#{@terminum-exit}|#{pane_pid}"]) {
            Ok(said) => said,
            Err(e) if gone(&e) => return Window::Gone,
            Err(_) => return Window::Unknown,
        };
        let mut fields = said.split('|');
        let (mark, exit, pid) = (fields.next().unwrap_or_default(), fields.next().unwrap_or_default(), fields.next().unwrap_or_default());
        if mark.is_empty() || label.is_some_and(|l| l != mark) {
            return Window::Gone;
        }
        Window::Ours { pane, status: exit.trim().parse().ok(), pid: pid.trim().parse().ok() }
    }
}

/// A window as `read` finds it.
enum Window<'a> {
    Ours { pane: &'a str, status: Option<i32>, pid: Option<i32> },
    Gone,
    Unknown,
}

/// tmux's words for a pane or a server that is no more; any other failure says nothing of the window.
fn gone(said: &str) -> bool {
    ["can't find pane", "can't find window", "no server running", "error connecting to"].iter().any(|w| said.contains(w))
}

impl Runner for Tmux {
    fn start(&self, label: &str, folder: &Path, line: &str, vars: &[(&str, String)]) -> Result<String, String> {
        let wrapper = crate::dispatcher::start::window_wrapper();
        let tmux = self.tmux.display().to_string();
        let folder = folder.display().to_string();
        let command = with_vars(line, vars);
        let argv = ["/bin/sh", "-c", wrapper, "sh", folder.as_str(), command.as_str(), tmux.as_str()];
        let made = if self.run(&["has-session", "-t", &format!("={SESSION}")]).is_ok() {
            let mut a = vec!["new-window", "-d", "-t", "=integration:", "-n", label, "-P", "-F", "#{pane_id}"];
            a.extend(argv);
            self.run(&a)
        } else {
            let mut a = vec!["new-session", "-d", "-s", SESSION, "-n", label, "-P", "-F", "#{pane_id}"];
            a.extend(argv);
            self.run(&a)
        }.map_err(|e| format!("No window for the line: {e}"))?;
        self.run(&["set-option", "-p", "-t", &made, "@terminum-integration", label]).map_err(|e| format!("tmux did not take the mark: {e}"))?;
        Ok(format!("{made} {label}"))
    }

    fn status(&self, handle: &str) -> Result<Option<i32>, String> {
        match self.read(handle) {
            Window::Ours { status, .. } => Ok(status),
            Window::Gone => Err("its window is gone".into()),
            // read again at the next beat: a line is never run again for a moment tmux did not answer
            Window::Unknown => Ok(None),
        }
    }

    fn output(&self, handle: &str) -> String {
        let Window::Ours { pane, .. } = self.read(handle) else { return String::new() };
        self.run(&["capture-pane", "-p", "-J", "-S", "-", "-E", "-", "-t", pane]).unwrap_or_default()
    }

    fn stop(&self, handle: &str) {
        let Window::Ours { pid: Some(pid), .. } = self.read(handle) else { return };
        // the wrapper runs the line in a process group of its own (set -m): its children's group is the line's
        let groups = child_groups(pid);
        let signal = |sig| groups.iter().for_each(|g| {
            // SAFETY: a signal to a process group this pane's wrapper made, read just now
            unsafe { libc::killpg(*g, sig) };
        });
        signal(libc::SIGTERM);
        // SIGKILL 10 s later, off the engine's step: its beat goes on meanwhile
        let late = groups.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(10));
            for g in late {
                // SAFETY: as above; a group that ended meanwhile gets ESRCH
                unsafe { libc::killpg(g, libc::SIGKILL) };
            }
        });
    }

    fn close(&self, handle: &str) {
        if let Window::Ours { pane, .. } = self.read(handle) {
            let _ = self.run(&["kill-pane", "-t", pane]);
        }
    }
}

/// The process groups of `pid`'s children, read from ps: the line's own, never the wrapper's.
fn child_groups(pid: i32) -> Vec<i32> {
    let Ok(o) = Command::new("ps").args(["-A", "-o", "pid=,ppid=,pgid="]).output() else { return Vec::new() };
    let mut groups: Vec<i32> = String::from_utf8_lossy(&o.stdout).lines()
        .filter_map(|l| { let v: Vec<i32> = l.split_whitespace().filter_map(|w| w.parse().ok()).collect(); (v.len() == 3 && v[1] == pid && v[2] != pid && v[2] > 1).then_some(v[2]) })
        .collect();
    groups.sort_unstable();
    groups.dedup();
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ac190_the_variables_go_before_the_line_quoted_and_the_line_as_typed() {
        let got = with_vars("make smoke && echo \"$TM_INTEGRATION_SHA\"", &[("TM_INTEGRATION_SHA", "3e9b5c2".into()), ("TM_INTEGRATION_TASKS", "API-1 it's".into())]);
        assert_eq!(got, "export TM_INTEGRATION_SHA=3e9b5c2; export TM_INTEGRATION_TASKS='API-1 it'\"'\"'s'; make smoke && echo \"$TM_INTEGRATION_SHA\"");
        let o = Command::new("/bin/sh").arg("-c").arg(format!("{}; printf %s \"$TM_INTEGRATION_TASKS\"", with_vars("true", &[("TM_INTEGRATION_TASKS", "API-1 it's".into())]))).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout), "API-1 it's", "a value's quote stays");
    }

    #[test]
    fn ac190_a_window_of_a_server_started_anew_is_never_read_as_the_lost_one() {
        let tmux = crate::testutil::Tmux::start("runner-anew", (80, 24), "sleep 120").expect("tmux is required for the runner's check");
        let runner = Tmux { tmux: "tmux".into(), socket: tmux.socket.clone(), conf: "/dev/null".into() };
        let check = runner.start("batch-2.1.check", &tmux.dir, "sleep 120", &[]).unwrap();
        assert!(check.ends_with(" batch-2.1.check"), "the handle keeps its mark: {check}");
        assert_eq!(runner.status(&check), Ok(None), "its own window runs");
        // the server ends (a crash, kill-server, a reboot) and starts anew: its pane IDs start again from %0
        tmux.tmux(&["kill-server"]);
        let other = (0..4).map(|n| runner.start(&format!("verify-{n}"), &tmux.dir, "exit 4", &[]).unwrap())
            .find(|h| pane_of(h) == pane_of(&check)).expect("a window of the new server takes the lost window's pane ID");
        assert!((0..100).any(|_| { std::thread::sleep(std::time::Duration::from_millis(50)); runner.status(&other) == Ok(Some(4)) }), "the other line ends 4");
        assert!(runner.status(&check).is_err(), "another line's window under the same ID is the check's lost window, never its status");
        runner.stop(&check);
        runner.close(&check);
        assert_eq!(runner.status(&other), Ok(Some(4)), "stopping or closing the lost check leaves the other line's window alone");
        assert_eq!(pane_of("%7"), "%7", "a handle kept before marks were in it is its pane");
        drop(tmux);
        assert!(runner.status(&other).is_err(), "no server: the window is gone");
    }

    #[test]
    fn ac190_only_tmux_s_words_for_a_pane_or_server_that_is_no_more_lose_a_window() {
        for said in ["can't find pane: %5", "no server running on /tmp/tmux-501/tm", "error connecting to /tmp/tmux-501/tm (No such file or directory)"] {
            assert!(gone(said), "{said}");
        }
        for said in ["tmux: Resource temporarily unavailable (os error 35)", "server exited unexpectedly", ""] {
            assert!(!gone(said), "a failure that says nothing of the window: {said}");
        }
    }
}
