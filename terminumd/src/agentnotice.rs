// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Windows when an agent starts where terminum serves it less well (`terminum-agents.md` AC-208): a
//! Codex under a shared app-server (AC-207) and Claude Code or Codex in its full-screen TUI. A page's
//! frames of a pane (page/stream.rs) have terminumd look when the pane opens and when its program,
//! screen or Claude Code's cursor changes; terminumd alone decides: the agent by the pane's program and
//! its foreground process's words, the run by that process's id, what the run was shown by the pane's
//! option `@tm-noticed` (it outlives terminumd and ends with the pane), what the user turned off by
//! settings.json's `agentNoticesOff`. Only this Mac's panes: the run and the shared server are its processes.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::app::Shared;
use crate::idle_exit::ask as tmux;
use crate::page::Page;
use crate::process::codex::{self, may_attach, NO_ATTACH, NO_DAEMON, OWN_OPTION, STOP_SERVER};
use crate::process::{foreground, program_at, Foreground};
use crate::screen::frame::Fields;
use crate::tmux::Server;

/// The kinds, as settings.json and the page name them.
pub const CODEX_SHARED: &str = "codexShared";
pub const FULL_SCREEN: &str = "fullScreen";
pub const KINDS: [&str; 2] = [CODEX_SHARED, FULL_SCREEN];

/// A Codex younger than this may not have started its own MCP servers yet: it is looked at once it is this old.
const SETTLED: u64 = 5;
/// How long Claude Code's hidden cursor must hold: a program redrawing hides it too (the page's `!`, AC-05).
const TUI_HOLD: Duration = Duration::from_millis(400);
/// What is read of the pane, tab-separated; the option last (the user's tmux may hold anything there).
const FIELDS: &str = "#{alternate_on}\t#{cursor_flag}\t#{pane_current_command}\t#{pane_tty}\t#{@tm-noticed}";

/// `claude` or `codex` when the pane's program is one (a `node` Codex too) and its foreground process's
/// words name the same; anything else is no agent of this case (vim, mc, a shell, OpenCode).
pub fn agent(command: &str, words: &[String]) -> Option<&'static str> {
    let named = crate::tmux::titles::agent_of(command);
    ["claude", "codex"].into_iter().find(|a| (named == Some(*a) || command == "node") && program_at(words, a).is_some())
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

/// Whether a pane's frame `new` (after `old`, none when the pane just opened) is worth a look: a coding
/// agent's program (or `node`) whose program or screen changed, or Claude Code's cursor on its alternate screen.
pub fn worth_a_look(old: Option<&Fields>, new: &Fields) -> bool {
    let candidate = |f: &Fields| crate::tmux::titles::agent_of(&f.command).is_some_and(|a| a == "claude" || a == "codex") || f.command == "node";
    let tui = |f: &Fields| f.alt && f.cursor_on && f.command == "claude";
    candidate(new) && old.is_none_or(|o| o.command != new.command || o.alt != new.alt || tui(o) != tui(new))
}

/// A look at the local pane `key` shown on `page` (its frames): the windows due, sent to that page. One at a
/// time a pane: a look still waiting (a young Codex) takes the asks that come meanwhile.
pub fn ask(app: &Shared, page: &Arc<Page>, server: &Arc<Server>, key: &str) {
    static LOOKING: Mutex<Option<HashSet<String>>> = Mutex::new(None);
    let Some(pane) = crate::page::trace::pane_of(key).filter(|_| server.is_local()) else { return };
    if !LOOKING.lock().get_or_insert_default().insert(key.to_string()) {
        return;
    }
    let (app, page, server, key) = (app.clone(), page.clone(), server.clone(), key.to_string());
    tokio::spawn(async move {
        if let Err(e) = look(&app, &page, &server, &pane, &key).await {
            tracing::debug!(event = "notice.unread", key, error = e);
        }
        LOOKING.lock().get_or_insert_default().remove(&key);
    });
}

/// The pane's fields (alternate screen, cursor, program, tty, mark) and its foreground run, read now.
async fn read(server: &Server, pane: &str) -> Result<(Vec<String>, Option<Foreground>), String> {
    let said = tmux(server, format!("display -p -t {pane} '{FIELDS}'")).await?;
    let f: Vec<String> = said.first().ok_or("no answer")?.splitn(5, '\t').map(str::to_string).collect();
    if f.len() < 5 {
        return Err("short answer".into());
    }
    let tty = f[3].clone();
    let run = tokio::task::spawn_blocking(move || foreground(&tty)).await.map_err(|e| e.to_string())?;
    Ok((f, run))
}

async fn look(app: &Shared, page: &Page, server: &Server, pane: &str, key: &str) -> Result<(), String> {
    let (mut f, mut run) = read(server, pane).await?;
    let wait = match run.as_ref().and_then(|r| agent(&f[2], &r.words).map(|a| (a, r))) {
        // a young Codex may not have started its own MCP servers yet; Claude Code's hidden cursor must hold
        Some(("codex", r)) if r.age < SETTLED && may_attach(&r.words) => Some(Duration::from_secs(SETTLED - r.age)),
        Some(("claude", _)) if f[0] == "1" && f[1] == "0" => Some(TUI_HOLD),
        _ => None,
    };
    if let Some(wait) = wait {
        let pid = run.as_ref().map(|r| r.pid);
        tokio::time::sleep(wait).await;
        (f, run) = read(server, pane).await?;
        if run.as_ref().map(|r| r.pid) != pid {
            return Ok(());
        }
    }
    let Some((agent, run)) = run.and_then(|r| agent(&f[2], &r.words).map(|a| (a, r))) else { return Ok(()) };
    // what this run was shown and what the user turned off go first: the look for a shared server reads every process
    let skip = |k: &&str| noticed(&f[4], run.pid).contains(k) || app.looks.settings.read().agent_notices_off.iter().any(|o| o == k);
    let (alt, cursor_on) = (f[0] == "1", f[1] == "1");
    let shared = agent == "codex" && !skip(&CODEX_SHARED) && may_attach(&run.words) && !run.children;
    let server_pid = if shared { tokio::task::spawn_blocking(codex::running).await.map_err(|e| e.to_string())? } else { None };
    let kinds: Vec<&str> = due(agent, alt, cursor_on, server_pid.is_some()).into_iter().filter(|k| !skip(k)).collect();
    if kinds.is_empty() {
        return Ok(());
    }
    // the mark read again, chosen and set one look at a time: two pages showing the pane get each window once
    static ONE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _one = ONE.lock().await;
    let mark = tmux(server, format!("display -p -t {pane} '#{{@tm-noticed}}'")).await?;
    let was = noticed(mark.first().map_or("", String::as_str), run.pid);
    let new: Vec<&str> = kinds.iter().copied().filter(|k| !was.contains(k)).collect();
    if new.is_empty() {
        return Ok(());
    }
    let all: Vec<&str> = KINDS.into_iter().filter(|k| was.contains(k) || kinds.contains(k)).collect();
    tmux(server, format!("set-option -p -t {pane} @tm-noticed '{} {}'", run.pid, all.join(","))).await?;
    let started = started(agent, &run.words);
    for kind in new {
        tracing::info!(event = "notice.shown", key, kind, pid = run.pid);
        page.send_json(&window(key, kind, agent, server_pid, &started));
    }
    Ok(())
}

/// How the run was started, to start it again: its program's name and its options, read as the idle exit's
/// resume reads them (AC-153: no prompt, no session it picked); the name alone where they cannot be read or hold a secret.
pub fn started(agent: &str, words: &[String]) -> Vec<String> {
    let opts = program_at(words, agent).and_then(|at| crate::agentopts::resumed(agent, &words[at + 1..])).unwrap_or_default();
    std::iter::once(agent.to_string()).chain(opts).collect()
}

/// The page's `notice` message: its words, each command a part of its own to copy, built on `started`.
pub fn window(key: &str, kind: &str, agent: &str, server_pid: Option<u32>, started: &[String]) -> Value {
    let text = |t: String| json!({"text": t});
    let cmd = |c: &str| json!({"cmd": c});
    // the run's words with `more` after them, every word quoted alike
    let again = |more: &str| {
        let words = started.iter().map(String::as_str).chain(more.split(' '));
        cmd(&crate::termtext::posix_line(words))
    };
    let (title, parts) = match (kind, agent) {
        (CODEX_SHARED, _) => ("This Codex runs without terminum", vec![
            text(format!("This Codex attached to a shared codex app-server{}, which a Codex app or an earlier Codex started. \
That server runs terminum's tools and hooks away from this pane, so terminum cannot tell this pane's Codex: no state or plan on its row, \
no task board, no terminum tools.", server_pid.map(|p| format!(" (process {p})")).unwrap_or_default())),
            text("Quit this Codex and start it again in its own process: as it was started, with one option more, either of:".into()),
            again(OWN_OPTION), again(NO_DAEMON),
            text("Or keep plain codex from attaching, then stop the server; stopping it ends the conversations it serves, and an app may start it again:".into()),
            cmd(NO_ATTACH), cmd(STOP_SERVER),
            text("terminum stops neither and changes no setting of Codex's.".into()),
        ]),
        (_, "claude") => {
            let mut parts = vec![
                text("In its full-screen TUI terminum shows only Claude Code's own screen: no history to scroll back, no widgets, folds or find \
over earlier output, no copy of earlier lines, no side marks.".into()),
                text("Switch it to its normal mode in Claude Code:".into()),
                cmd("/tui classic"),
            ];
            // Claude Code keeps only the last --settings: one of the run's own would be lost (terminum-dispatcher.md AC-156)
            if !crate::tasks::agents::names_settings(&started.join(" ")) {
                parts.extend([text("or quit it and start it again as it was started, in its normal mode:".into()), again(r#"--settings {"tui":"default"}"#)]);
            }
            parts.push(text("or set \"tui\": \"default\" in ~/.claude/settings.json and start it again.".into()));
            ("Claude Code runs full screen", parts)
        }
        _ => ("Codex runs full screen", vec![
            text("This Codex draws on tmux's alternate screen, where tmux keeps no scrollback: terminum cannot scroll back to, find or copy \
its earlier output.".into()),
            text("Quit it and start it again inline, as it was started with one option more (--no-alt-screen would attach it to a shared server where one runs):".into()),
            again(OWN_OPTION),
            text("or set alternate_screen = \"never\" under [tui] in ~/.codex/config.toml.".into()),
        ]),
    };
    json!({"t": "notice", "id": key, "kind": kind, "title": title, "parts": parts})
}

#[cfg(test)]
#[path = "agentnotice_tests.rs"]
mod tests;
