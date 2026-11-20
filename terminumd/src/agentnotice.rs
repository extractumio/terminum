// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Windows when an agent starts where terminum serves it less well (`terminum-agents.md` AC-208): a
//! Codex under a shared app-server (AC-207) and Claude Code or Codex in its full-screen TUI. The page
//! asks (`notice {id}`) when it shows a pane and when its program or screen changes; terminumd alone
//! decides: the agent by the pane's program and its foreground process's words, the run by that
//! process's id, what the run was shown by the pane's option `@tm-noticed` (it outlives terminumd and
//! ends with the pane), what the user turned off by settings.json's `agentNoticesOff`. Only this Mac's
//! panes: the run and the shared server are its processes.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use crate::app::Shared;
use crate::cli::codexdaemon::{self, NO_ATTACH, OWN_PROCESS, STOP_SERVER};
use crate::cli::hookargv::{exact, program_at};
use crate::page::Page;
use crate::tmux::Server;

/// The kinds, as settings.json and the page name them.
pub const CODEX_SHARED: &str = "codexShared";
pub const FULL_SCREEN: &str = "fullScreen";
pub const KINDS: [&str; 2] = [CODEX_SHARED, FULL_SCREEN];

/// A Codex younger than this may not have started its MCP servers yet: it is looked at once it is this old.
const SETTLED: u64 = 5;
/// What is read of the pane, tab-separated; the option last (the user's tmux may hold anything there).
const FIELDS: &str = "#{alternate_on}\t#{cursor_flag}\t#{pane_current_command}\t#{pane_tty}\t#{@tm-noticed}";
/// Codex's subcommands without its TUI.
const NO_TUI: [&str; 2] = ["exec", "e"];

/// The pane's foreground process (its tty's foreground group's leader).
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub pid: u32,
    pub words: Vec<String>,
    /// seconds it has run
    pub age: u64,
    /// a process of its own under it (a Codex in its own process has its MCP servers there)
    pub children: bool,
}

/// `claude` or `codex` when the pane's program is one (a `node` Codex too) and its foreground process's
/// words name the same; anything else is no agent of this case (vim, mc, a shell, OpenCode).
pub fn agent(command: &str, words: &[String]) -> Option<&'static str> {
    let named = crate::tmux::titles::agent_of(command);
    ["claude", "codex"].into_iter().find(|a| (named == Some(*a) || command == "node") && program_at(words, a).is_some())
}

/// Whether a Codex started with `words` attaches to a shared server when one runs: no `-c`, `--config` or
/// `--no-daemon` (each keeps it in its own process, AS-128), and its TUI (not `codex exec`).
pub fn may_attach(words: &[String]) -> bool {
    let Some(at) = program_at(words, "codex") else { return false };
    let rest = &words[at + 1..];
    let own = rest.iter().any(|w| w.starts_with("-c") || w == "--config" || w.starts_with("--config=") || w == "--no-daemon");
    let first = rest.iter().find(|w| !w.starts_with('-'));
    !own && !first.is_some_and(|w| NO_TUI.contains(&w.as_str()))
}

/// The kinds due for a run of `agent`: full screen on the alternate screen (Claude Code's with the
/// terminal's cursor hidden: its Ctrl+G editor shows it, AC-05; Codex shows it in its own box), and
/// `shared` (a Codex under a shared server).
pub fn due(agent: &str, alt: bool, cursor_on: bool, shared: bool) -> Vec<&'static str> {
    let full = alt && (agent == "codex" || !cursor_on);
    [(shared && agent == "codex", CODEX_SHARED), (full, FULL_SCREEN)].into_iter().filter(|(on, _)| *on).map(|(_, k)| k).collect()
}

/// The kinds `@tm-noticed` (`<pid> <kind>,<kind>`) says run `pid` was shown.
pub fn noticed(value: &str, pid: u32) -> Vec<&'static str> {
    match value.trim().split_once(' ') {
        Some((p, kinds)) if p.parse() == Ok(pid) => KINDS.into_iter().filter(|k| kinds.split(',').any(|s| s == *k)).collect(),
        _ => Vec::new(),
    }
}

/// `[[dd-]hh:]mm:ss`, as `ps -o etime` says it, in seconds.
fn seconds(etime: &str) -> Option<u64> {
    let (days, rest) = etime.split_once('-').map_or(Ok((0, etime)), |(d, r)| d.parse().map(|d: u64| (d, r))).ok()?;
    rest.split(':').try_fold(0u64, |s, part| part.parse::<u64>().ok().map(|n| s * 60 + n)).map(|s| s + days * 86_400)
}

/// The foreground run in `ps -o pid=,ppid=,tpgid=,etime=` rows of one tty.
pub fn foreground_of(rows: &str) -> Option<(u32, u64, bool)> {
    let rows: Vec<[&str; 4]> = rows.lines().filter_map(|l| {
        let f: Vec<&str> = l.split_whitespace().collect();
        <[&str; 4]>::try_from(f).ok()
    }).collect();
    let lead = rows.iter().find(|r| r[0] == r[2])?;
    let pid = lead[0].parse().ok()?;
    Some((pid, seconds(lead[3])?, rows.iter().any(|r| r[1] == lead[0])))
}

/// The foreground run of terminal `tty` (`/dev/ttys004`), by `ps` and the kernel's words.
fn foreground(tty: &str) -> Option<Run> {
    let name = tty.strip_prefix("/dev/").filter(|n| !n.is_empty() && !n.contains(['/', ' ']))?;
    let out = std::process::Command::new("ps").args(["-t", name, "-o", "pid=,ppid=,tpgid=,etime="]).output().ok()?;
    let (pid, age, children) = foreground_of(&String::from_utf8_lossy(&out.stdout))?;
    Some(Run { pid, words: exact(pid)?, age, children })
}

/// What the page that asked about the shown pane `id` is to show: the pane's windows due, if any.
pub fn ask(app: &Shared, page: &Arc<Page>, id: &str) {
    let shown = page.state.lock().shown.as_ref().filter(|s| s.key == id).map(|s| (s.server.clone(), s.pane.clone()));
    let Some((server, pane)) = shown.filter(|(s, _)| s.is_local()) else { return };
    let (app, page, key) = (app.clone(), page.clone(), id.to_string());
    tokio::spawn(async move {
        if let Err(e) = look(&app, &page, &server, &pane, &key).await {
            tracing::debug!(event = "notice.unread", key, error = e);
        }
    });
}

/// The pane's fields and its foreground run, read now.
async fn read(server: &Server, pane: &str) -> Result<(Vec<String>, Option<Run>), String> {
    let said = server.run(format!("display -p -t {pane} '{FIELDS}'")).await?;
    let f: Vec<String> = said.first().ok_or("no answer")?.splitn(5, '\t').map(str::to_string).collect();
    if f.len() < 4 {
        return Err("short answer".into());
    }
    let tty = f[3].clone();
    let run = tokio::task::spawn_blocking(move || foreground(&tty)).await.map_err(|e| e.to_string())?;
    Ok((f, run))
}

async fn look(app: &Shared, page: &Page, server: &Server, pane: &str, key: &str) -> Result<(), String> {
    let (mut f, mut run) = read(server, pane).await?;
    let mut agent_now = run.as_ref().and_then(|r| agent(&f[2], &r.words));
    // a young Codex may not have started its own MCP servers yet: looked at again once it settled
    if let (Some("codex"), Some(r)) = (agent_now, &run) && r.age < SETTLED && may_attach(&r.words) {
        let pid = r.pid;
        tokio::time::sleep(Duration::from_secs(SETTLED - r.age)).await;
        (f, run) = read(server, pane).await?;
        if run.as_ref().map(|r| r.pid) != Some(pid) {
            return Ok(());
        }
        agent_now = run.as_ref().and_then(|r| agent(&f[2], &r.words));
    }
    let (Some(agent), Some(run)) = (agent_now, run) else { return Ok(()) };
    let server_pid = if agent == "codex" && may_attach(&run.words) && !run.children {
        tokio::task::spawn_blocking(codexdaemon::running).await.map_err(|e| e.to_string())?
    } else {
        None
    };
    let off = app.looks.settings.read().agent_notices_off.clone();
    let kinds: Vec<&str> = due(agent, f[0] == "1", f[1] == "1", server_pid.is_some()).into_iter().filter(|k| !off.iter().any(|o| o == k)).collect();
    if kinds.is_empty() {
        return Ok(());
    }
    // the read, the choice and the mark one ask at a time: two pages asking at once get it once
    static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ONE.lock().await;
    let mark = server.run(format!("display -p -t {pane} '#{{@tm-noticed}}'")).await?;
    let was = noticed(mark.first().map_or("", String::as_str), run.pid);
    let new: Vec<&str> = kinds.into_iter().filter(|k| !was.contains(k)).collect();
    if new.is_empty() {
        return Ok(());
    }
    let all: Vec<&str> = KINDS.into_iter().filter(|k| was.contains(k) || new.contains(k)).collect();
    server.run(format!("set-option -p -t {pane} @tm-noticed '{} {}'", run.pid, all.join(","))).await?;
    for kind in new {
        tracing::info!(event = "notice.shown", key, kind, pid = run.pid);
        page.send_json(&window(key, kind, agent, server_pid));
    }
    Ok(())
}

/// The page's `notice` message: its words, each command a part of its own to copy.
pub fn window(key: &str, kind: &str, agent: &str, server_pid: Option<u32>) -> Value {
    let text = |t: String| json!({"text": t});
    let cmd = |c: &str| json!({"cmd": c});
    let (title, parts) = match (kind, agent) {
        (CODEX_SHARED, _) => ("This Codex runs without terminum", vec![
            text(format!("This Codex attached to a shared codex app-server{}, which a Codex app or an earlier Codex started. \
That server runs terminum's tools and hooks away from this pane, so terminum cannot tell this pane's Codex: no state or plan on its row, \
no task board, no terminum tools.", server_pid.map(|p| format!(" (process {p})")).unwrap_or_default())),
            text("Quit this Codex and start it again in its own process, one of (leave out an option an alias of yours already has):".into()),
            cmd(OWN_PROCESS), cmd("codex --no-daemon"),
            text("Or keep plain codex from attaching, then stop the server; stopping it ends the conversations it serves, and an app may start it again:".into()),
            cmd(NO_ATTACH), cmd(STOP_SERVER),
            text("terminum stops neither and changes no setting of Codex's.".into()),
        ]),
        (_, "claude") => ("Claude Code runs full screen", vec![
            text("In its full-screen TUI terminum shows only Claude Code's own screen: no history to scroll back, no widgets, folds or find \
over earlier output, no copy of earlier lines, no side marks.".into()),
            text("Switch it to its normal mode in Claude Code:".into()),
            cmd("/tui classic"),
            text("or set \"tui\": \"default\" in ~/.claude/settings.json and start it again.".into()),
        ]),
        _ => ("Codex runs full screen", vec![
            text("This Codex draws on tmux's alternate screen, where tmux keeps no scrollback: terminum cannot scroll back to, find or copy \
its earlier output.".into()),
            text("Quit it and start it inline (--no-alt-screen would attach it to a shared server where one runs):".into()),
            cmd(OWN_PROCESS),
            text("or set alternate_screen = \"never\" under [tui] in ~/.codex/config.toml.".into()),
        ]),
    };
    json!({"t": "notice", "sid": key, "kind": kind, "title": title, "parts": parts})
}

#[cfg(test)]
#[path = "agentnotice_tests.rs"]
mod tests;
