// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Where a project's Check, Publish and After run (`terminum-integration.md` AC-190, §9 "Commands"): each line in a
//! window of terminumd's own tmux server, by the user's login shell with the batch's variables, marked
//! `@terminum-integration`; Dispatcher's window wrapper writes the line's status as `@terminum-exit` and keeps the
//! pane, so a check outlives a restart of terminumd and its status and output are read after. Nothing is typed in.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Runs a line and follows it; a test stands in for tmux.
pub trait Runner: Send + Sync {
    /// Starts `line` in `folder` with `vars`; its handle (a pane).
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

/// A line's words for the shell: each value one single-quoted word.
pub fn sh_quote(v: &str) -> String {
    format!("'{}'", v.replace('\'', r"'\''"))
}

/// `export K='v'; …; line`: the variables before the human's line, which runs as typed.
pub fn with_vars(line: &str, vars: &[(&str, String)]) -> String {
    let mut out: String = vars.iter().map(|(k, v)| format!("export {k}={}; ", sh_quote(v))).collect();
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

impl Tmux {
    fn run(&self, args: &[&str]) -> Result<String, String> {
        let o = Command::new(&self.tmux).args(["-L", &self.socket, "-f"]).arg(&self.conf).args(args).output().map_err(|e| format!("tmux: {e}"))?;
        if !o.status.success() {
            return Err(String::from_utf8_lossy(&o.stderr).trim().to_string());
        }
        Ok(String::from_utf8_lossy(&o.stdout).trim().to_string())
    }
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
        Ok(made)
    }

    fn status(&self, pane: &str) -> Result<Option<i32>, String> {
        let said = self.run(&["display", "-p", "-t", pane, "#{@terminum-integration}|#{@terminum-exit}"]).map_err(|_| "its window is gone".to_string())?;
        let (mark, exit) = said.split_once('|').unwrap_or((&said, ""));
        if mark.is_empty() {
            return Err("its window is gone".into());   // a pane of another's: never read as ours
        }
        Ok(exit.trim().parse::<i32>().ok())
    }

    fn output(&self, pane: &str) -> String {
        self.run(&["capture-pane", "-p", "-J", "-S", "-", "-E", "-", "-t", pane]).unwrap_or_default()
    }

    fn stop(&self, pane: &str) {
        let Ok(pid) = self.run(&["display", "-p", "-t", pane, "#{pane_pid}"]) else { return };
        let Ok(pid) = pid.parse::<i32>() else { return };
        // the wrapper runs the line in a process group of its own (set -m): its children's group is the line's
        let groups = child_groups(pid);
        for sig in [libc::SIGTERM, libc::SIGKILL] {
            for g in &groups {
                // SAFETY: a signal to a process group this pane's wrapper made, read just now
                unsafe { libc::killpg(*g, sig) };
            }
            if sig == libc::SIGTERM {
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        }
    }

    fn close(&self, pane: &str) {
        if self.status(pane).is_ok() {
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
        assert_eq!(got, "export TM_INTEGRATION_SHA='3e9b5c2'; export TM_INTEGRATION_TASKS='API-1 it'\\''s'; make smoke && echo \"$TM_INTEGRATION_SHA\"");
        let o = Command::new("/bin/sh").arg("-c").arg(format!("{}; printf %s \"$TM_INTEGRATION_TASKS\"", with_vars("true", &[("TM_INTEGRATION_TASKS", "API-1 it's".into())]))).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&o.stdout), "API-1 it's", "a value's quote stays");
    }
}
