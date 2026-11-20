// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A shared `codex app-server` (`terminum-agents.md` AC-207): one a Codex app or editor started,
//! listening on a socket, to which a Codex started by hand attaches by itself. It starts that
//! Codex's MCP servers and runs its hooks in its own environment, not the pane's, so terminum
//! cannot tell which pane the agent is in; a TMUX_PANE it has is some other pane's. Under one,
//! `terminum mcp` offers only a tool saying why (mcp.rs) and `terminum hook` sends nothing
//! (hook.rs); `terminum hooks` and its install say when one runs (agentsetup.rs). terminum never
//! stops it and never changes Codex's configuration for it.

use std::process::Command;

/// The fix's commands, as Codex 0.170 takes them; a `-c` override keeps every Codex since 0.167 in its own
/// process (`terminum-dispatcher.md` AS-128), and this one also keeps it inline (AC-208).
macro_rules! own_process { () => { "codex -c tui.alternate_screen=never" }; }
macro_rules! no_attach { () => { "codex features disable daemon_auto_start" }; }
macro_rules! stop_server { () => { "codex app-server daemon stop" }; }

/// The fix, in words.
macro_rules! fix {
    () => {
        concat!("start Codex in the pane with a -c override (", own_process!(), ") or --no-daemon; to keep \
plain codex from attaching, run \"", no_attach!(), "\", then \"", stop_server!(), "\" \
(it ends the conversations that server serves)")
    };
}

/// Starts a Codex in its own process, inline (AC-208's windows).
pub const OWN_PROCESS: &str = own_process!();
/// Keeps plain `codex` from attaching, then stops the server (AC-208's window).
pub const NO_ATTACH: &str = no_attach!();
pub const STOP_SERVER: &str = stop_server!();

/// The description of the one tool a session under a shared app-server is offered, and its answer.
pub const SAID: &str = concat!("terminum cannot see which terminal this Codex runs in: it runs under a shared \
codex app-server (one a Codex app or editor started), which starts terminum's MCP server and hooks without the \
pane's TMUX and TMUX_PANE, so terminum shows no state or plan for it. If this Codex was started in a terminum \
pane, ", fix!(), ". Otherwise go on without terminum's tools.");

/// The one tool a session under a shared app-server is offered: its name says the question.
pub const TOOL: &str = "why_no_terminum_tools";

/// The ancestors looked at: the agent's program may run a hook through a shell.
const UP: usize = 3;

/// Whether `words` (a command line) are a Codex app-server listening where other Codex
/// processes attach: `--listen unix://…` or `ws://…` (a stdio one is its client's alone).
fn shared(words: &[impl AsRef<str>]) -> bool {
    let w: Vec<&str> = words.iter().map(AsRef::as_ref).collect();
    let program = super::hookargv::program_at(&w, "codex").is_some();
    let listens = |url: &str| url.starts_with("unix://") || url.starts_with("ws://");
    let listening = w.iter().enumerate().any(|(i, a)| match a.strip_prefix("--listen") {
        Some("") => w.get(i + 1).is_some_and(|u| listens(u)),
        Some(rest) => rest.strip_prefix('=').is_some_and(listens),
        None => false,
    });
    program && w.contains(&"app-server") && listening
}

/// Whether a shared app-server started this process: one of its nearest ancestors' exact words
/// (the kernel's, no `ps`: a hook pays microseconds); a Codex of its own (the agent) ends the look.
pub fn under_shared() -> bool {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..UP {
        if let Some(words) = super::hookargv::exact(pid) && super::hookargv::program_at(&words, "codex").is_some() {
            return shared(&words);
        }
        match parent_of(pid) {
            Some(p) if p > 1 => pid = p,
            _ => break,
        }
    }
    false
}

/// Process `pid`'s parent.
#[cfg(target_os = "macos")]
fn parent_of(pid: u32) -> Option<u32> {
    // SAFETY: proc_bsdinfo is plain data; proc_pidinfo fills at most `size` bytes of it
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_bsdinfo>() as libc::c_int;
    let got = unsafe { libc::proc_pidinfo(libc::c_int::try_from(pid).ok()?, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    (got == size).then_some(info.pbi_ppid)
}

/// Process `pid`'s parent: the field after the state, past the command's closing parenthesis.
#[cfg(target_os = "linux")]
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(1)?.parse().ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn parent_of(_pid: u32) -> Option<u32> {
    None
}

/// The process id of a shared app-server of this user that runs, if one does (`ps`, once).
pub fn running() -> Option<u32> {
    // SAFETY: getuid has no preconditions
    let uid = unsafe { libc::getuid() }.to_string();
    let out = Command::new("ps").args(["-ww", "-U", &uid, "-o", "pid=,args="]).output().ok()?;
    String::from_utf8_lossy(&out.stdout).lines().find_map(|row| {
        let mut words = row.split_whitespace();
        let pid = words.next()?.parse().ok()?;
        shared(&words.collect::<Vec<_>>()).then_some(pid)
    })
}

/// The line `terminum hooks` and its install say when a shared app-server runs.
pub fn warning() -> Option<String> {
    running().map(|pid| format!("Codex: a shared codex app-server runs (process {pid}); a Codex started in a pane \
may attach to it, and terminum then gets no hooks, tools, state or plan from that Codex. To avoid it, {}; \
terminum stops neither.", fix!()))
}

#[cfg(test)]
#[path = "codexdaemon_tests.rs"]
mod tests;
