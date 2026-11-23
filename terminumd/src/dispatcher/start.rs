// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! RUN (`terminum-dispatcher.md` AC-121, AC-122): opens the window with the fixed wrapper of §9,
//! marks its pane, and watches the agent's box until the prompt is taken.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::runs::{self, Phase, Run};
use super::screen::Screen;
use super::{gate, hooks, move_task, say, store};
use crate::app::Shared;
use crate::tasks::ops;
use crate::tasks::store::DISPATCHER;
use crate::tmux::server::{Link, HOST_SESSION};
use crate::tmux::{args, Server};

pub(super) const BOX_WITHIN: Duration = Duration::from_secs(60);
const TAKEN_WITHIN: Duration = Duration::from_secs(30);
/// How long an empty box stands before a first prompt goes in with no start heard.
const STANDS: Duration = Duration::from_secs(2);
/// A test's clock runs these slower than the rest: a program needs its real moment to start.
pub(super) fn start_scaled(d: Duration) -> Duration {
    d / super::scale().min(10) as u32
}
/// The wrapper's words are its arguments: no quote or `#` in them can break it. What the tty still
/// holds when the agent has gone is dropped, so nothing meant for it reaches the shell it leaves.
/// `set -m` takes the terminal back whichever way the agent went: left with a dead process group, an
/// older bash's login shell (macOS's) or ash's waits for the terminal for good, spinning, with no prompt.
/// The `trap`: with job control a dash or ash dies of its job's Ctrl-C, and the window with it; a caught
/// signal goes back to its default in the agent and the shell after.
const WRAPPER: &str = r#"trap : INT; set -m; d=$1; c=$2; t=$3; case $d in "~"|"~/"*) d=$HOME${d#"~"};; esac; s=${SHELL:-/bin/sh}; if cd -- "$d"; then "$s" -lic "$c"; e=$?; else e=-1; fi; "$t" set -p -t "$TMUX_PANE" @terminum-exit "$e"; if [ "$4" = "round" ]; then exit 0; fi; stty -icanon min 0 time 0 2>/dev/null; cat >/dev/null 2>&1; stty sane 2>/dev/null; exec "$s" -l"#;
/// The wrapper a window's line runs in: its status as `@terminum-exit`, the pane kept (terminum-integration.md AC-190).
pub(crate) fn window_wrapper() -> &'static str {
    WRAPPER
}

/// What the wrapper records when it could not enter the folder: no process exits with it.
const NO_FOLDER: i32 = -1;

/// The words of a shell-command's arguments for tmux: each quoted, tmux runs no shell of its own.
fn window_command(folder: &str, command: &str, tmux: &str, round: bool) -> String {
    let mode = if round { "round" } else { "" };
    ["/bin/sh", "-c", WRAPPER, "sh", folder, command, tmux, mode].iter().map(|w| args::quote(w)).collect::<Vec<_>>().join(" ")
}

fn tmux_of(server: &Server) -> String {
    match &server.link {
        Link::Local { tmux, .. } => tmux.display().to_string(),
        Link::Ssh { .. } => "tmux".to_string(),
    }
}

/// A new window: after the project's last session on this server, else in a session of its name,
/// made when there is none; its pane and where it stands.
async fn open_window(app: &Shared, server: &Arc<Server>, project: Option<(&str, &str)>, folder: &str, command: &str, staged: bool) -> Result<(String, String), String> {
    let cmd = if staged { "/bin/sleep 120".into() } else { window_command(folder, command, &tmux_of(server), false) };
    let fmt = "-P -F '#{pane_id} #{session_name}:#{window_index}'";
    let last = project.and_then(|(id, _)| {
        let ps = app.projects.lock();
        ps.by_id(id).and_then(|p| p.members.iter().rev().filter_map(|k| app.tmux.resolve(k)).find(|(s, _)| Arc::ptr_eq(s, server)).map(|(_, pane)| pane))
    });
    let window = match &last {
        Some(pane) => crate::page::sessions::window_of(server, pane).await.ok(),
        None => None,
    };
    let line = if let Some(w) = window {
        format!("new-window -d -a -t {w} {fmt} {cmd}")
    } else if !server.is_local() {
        // a host shows only the session terminumd holds there (terminum-remote.md OQ-81)
        format!("new-window -d -t {} {fmt} {cmd}", args::quote(&format!("={HOST_SESSION}:")))
    } else {
        let wanted = args::project_session(project.map_or("tasks", |(_, name)| name))?;
        let sessions = server.run("list-sessions -F '#{session_name}'").await.unwrap_or_default();
        let target = if sessions.contains(&wanted) { Some(wanted.clone()) } else if project.is_none() { sessions.first().cloned() } else { None };
        match target {
            Some(s) => format!("new-window -d -t {} {fmt} {cmd}", args::quote(&format!("={s}:"))),
            None => format!("new-session -d -s {} {fmt} {cmd}", args::format_arg(&wanted)),
        }
    };
    let made = server.run(line).await.map_err(|e| format!("No new window: {e}"))?;
    let first = made.into_iter().next().unwrap_or_default();
    first.split_once(' ').map(|(p, w)| (p.to_string(), w.to_string())).ok_or_else(|| "No new window: tmux named no pane".to_string())
}

/// Marks `pane` as `run`'s: only that is ever typed in, signalled or closed (AC-125).
async fn mark(server: &Server, pane: &str, run: &str) -> Result<(), String> {
    server.run(format!("set-option -p -t {pane} @terminum-run {run}")).await.map(drop).map_err(|e| format!("tmux did not take the mark: {e}"))?;
    let _ = server.run(format!("set-option -p -u -t {pane} @terminum-exit")).await;
    Ok(())
}

/// Whether `pane` still carries `run`'s mark: read just before any act on it.
pub async fn is_marked(server: &Server, pane: &str, run: &str) -> Result<(), String> {
    let mine = server.run(format!("display -p -t {pane} '#{{@terminum-run}}'")).await.map_err(|e| format!("tmux does not answer: {e}"))?;
    if mine.first().map(String::as_str) == Some(run) { Ok(()) } else { Err("its pane is not this run's".to_string()) }
}

/// What a window is for.
pub struct Opening<'a> {
    pub server: &'a Arc<Server>,
    /// its project's id and name, for the window's place in the list; none in Default
    pub project: Option<(&'a str, &'a str)>,
    pub folder: &'a str,
    pub command: &'a str,
    pub run: &'a str,
    pub tasks: &'a [String],
    pub program: &'a str,
    /// the agent's ID
    pub agent: &'a str,
}

/// Opens the window, links it and says so: its key.
pub async fn open(app: &Shared, o: Opening<'_>) -> Result<String, String> {
    open_pane(app, o, false).await
}

/// AC-234: no unmarked program runs while the store and tmux are recording the pane.
pub(super) async fn open_supervisor(app: &Shared, o: Opening<'_>) -> Result<String, String> {
    open_pane(app, o, true).await
}

async fn open_pane(app: &Shared, o: Opening<'_>, staged: bool) -> Result<String, String> {
    let (pane, place) = open_window(app, o.server, o.project, o.folder, o.command, staged).await?;
    if !staged {
        crate::inputs::wrote(app, crate::inputs::Who::dispatcher("dispatcher.start", o.run, o.tasks, o.agent), "launch", o.server, &pane, o.command, None, o.project);
    }
    mark(o.server, &pane, o.run).await?;
    if let Some((session, _)) = place.rsplit_once(':') {
        o.server.note_pane(&pane, session);            // its agent's first hook may come before the next list
    }
    let key = o.server.key(&pane);
    let project = o.project.map_or(crate::tasks::store::DEFAULT_PROJECT, |(id, _)| id);
    app.dispatcher.track_pane(&key, o.run, project);    // its agent's first hooks
    let on = if o.server.is_local() { "on this Mac".to_string() } else { format!("on {}", o.server.name) };
    let (a, run, tasks, words, k) = (app.clone(), o.run.to_string(), o.tasks.to_vec(), format!("Session started: {} in window {place}, {on}.", o.program), key.clone());
    let linked = store(app, move |tx| {
        for task in &tasks {
            ops::link(tx, DISPATCHER, task, Some(&k), ops::now())?;
        }
        if staged {
            crate::supervisor::rounds::mark(tx, &run, &k)?;
            runs::change(tx, &run, |r| r.pane_key = Some(k.clone()))?;
            return Ok(false);
        }
        let (r, released) = a.dispatcher.pane_opened(tx, &run, &k)?;
        say(tx, &r, &words)?;
        Ok(released)
    }).await;
    match linked {
        Ok(true) => app.supervisor.incident_changed(),
        Ok(false) => {}
        Err(why) => {
            if staged { let _ = marked_command(o.server, &pane, o.run, &format!("kill-pane -t {pane}")).await; }
            return Err(why);
        }
    }
    // a member only once its run names it: a project's pane no run names is a human's, its manager (AC-174)
    if let Some((project, _)) = o.project {
        let (a, k, project) = (app.clone(), key.clone(), project.to_string());
        let _ = crate::http::off(move || a.projects.lock().add_to(&k, &project, None)).await;
        crate::page::projects::tell(app);
    }
    if staged {
        let run = o.run.to_owned();
        store(app, move |tx| crate::supervisor::rounds::check_first_prompt(tx, &run)).await?;
        let line = format!("respawn-pane -k -t {pane} {}", window_command(o.folder, o.command, &tmux_of(o.server), true));
        marked_command(o.server, &pane, o.run, &line).await?;
        crate::inputs::wrote(app, crate::inputs::Who::dispatcher("dispatcher.start", o.run, o.tasks, o.agent),
            "launch", o.server, &pane, o.command, None, o.project);
    }
    Ok(key)
}

/// Test the mark in tmux itself, in the command that acts on it.
pub(super) async fn marked_command(server: &Server, pane: &str, run: &str, line: &str) -> Result<(), String> {
    let client = server.client(pane).ok_or("The pane's tmux client is unavailable.")?;
    gate::guarded(&client, run, pane, line, false).await
}

enum StartMiss<'a> {
    Exit(i32),
    Other(&'a str),
}

fn missing_codex_session<'a>(run: &'a Run, code: i32, rows: &[String]) -> Option<&'a str> {
    if run.kind != "codex" || code <= 0 {
        return None;
    }
    let id = run.agent_session.as_deref().filter(|id| hooks::good_id(id))?;
    let error = format!("ERROR: No saved session found with ID {id}");
    rows.iter().any(|row| {
        row.trim().strip_prefix(&error).is_some_and(|tail| tail.is_empty() || tail.starts_with(['.', ' ']))
    }).then_some(id)
}

/// Keep the conversation available for a retry. The exited pane stays open for inspection.
fn codex_resume_missing(tx: &rusqlite::Transaction, run: &mut Run, words: &str) -> Result<(), String> {
    let at = ops::now();
    for task in &run.open {
        if matches!(ops::get(tx, task)?.status.as_str(), "TODO" | "IN PROGRESS") {
            ops::move_to(tx, DISPATCHER, task, "WAIT FOR HUMAN", at)?;
        }
    }
    run.state = Phase::Closed;
    run.prompt = None;
    runs::save(tx, run)?;
    say(tx, run, words)
}

/// Report a start error with the screen's last rows, redacted; the window stays (AC-121, AC-122).
async fn did_not_start(app: &Shared, r: &Run, at: Option<(&Server, &str)>, miss: StartMiss<'_>) {
    let rows = match at {
        Some((s, pane)) => s.run(format!("capture-pane -p -J -t {pane}")).await.unwrap_or_default(),
        None => Vec::new(),
    };
    let tail: Vec<&str> = rows.iter().map(String::as_str).rev().skip_while(|l| l.trim().is_empty()).take(20).collect::<Vec<_>>().into_iter().rev().collect();
    let (exit, why) = match miss {
        StartMiss::Exit(NO_FOLDER) => (None, format!("The folder {} does not exist there", r.folder)),
        StartMiss::Exit(code) => (Some(code), format!("the program ended with exit {code}")),
        StartMiss::Other(why) => (None, why.to_string()),
    };
    let shown = crate::sent::redact(&tail.join("\n"));
    let words = if shown.is_empty() { format!("Session did not start: {why}") } else { format!("Session did not start: {why}\n\n```\n{shown}\n```") };
    if r.ad_hoc() {                                    // no task to say it on: the pages say it (AC-142)
        super::acts::note(app, format!("{} did not start: {why}", r.kind));
    }
    let id = r.id.clone();
    let done = store(app, move |tx| {
        let mut now = runs::by_id(tx, &id)?;
        if !now.is_starting() {
            return Ok(());                            // its first hook or task tool accepted it while the screen was read
        }
        if let Some(session) = exit.and_then(|code| missing_codex_session(&now, code, &rows)) {
            let words = format!("Codex could not find saved conversation {session}. Restore it and press RUN to try again, or choose Run from scratch.");
            return codex_resume_missing(tx, &mut now, &words);
        }
        now.gone();
        now.prompt = None;
        // Codex stores only a conversation carried into this run or confirmed by a prompt.
        if now.kind != "codex" {
            now.agent_session = None;
        }
        runs::save(tx, &now)?;
        super::fail(tx, &now, &words)
    }).await;
    if let Err(e) = done {
        tracing::warn!(event = "dispatcher.start", error = %e);
    }
    super::told_all(app, r);
}

/// Whether an empty box takes a first prompt now: Claude Code's SessionStart comes after its box; one that stands this
/// long has started, should the hook have gone unheard: its prompt's hook is what proves them (AS-127). `since`: when
/// it was first seen empty, kept by the caller across its looks.
pub(super) fn ready(app: &Shared, server: &Server, pane: &str, since: &mut Option<Instant>) -> bool {
    let at = *since.get_or_insert_with(Instant::now);
    app.hooks.session(&server.key(pane)).is_some() || at.elapsed() >= STANDS
}

/// The window command's record for `pane`, if the wrapper got there.
pub async fn exit_of(server: &Server, pane: &str) -> Option<i32> {
    server.run(format!("display -p -t {pane} '#{{@terminum-exit}}'")).await.ok()?.first()?.parse().ok()
}

/// Watches a run until its prompt is taken: the box, the gate, the hook. Started once per run; a
/// restart's walk starts it anew and reads every fact from the store.
pub fn watch(app: &Shared, id: &str) {
    let Some(held) = super::Held::take(app, |d| &d.starting, id) else { return };
    let (app, id) = (app.clone(), id.to_string());
    tokio::spawn(async move {
        watching(&app, &id).await;
        drop(held);
    });
}

/// Its own question stands before the box: nothing is typed, a human is asked (AC-121).
async fn now_asking(app: &Shared, r: &Run) {
    let id = r.id.clone();
    let moved = store(app, move |tx| {
        if !runs::by_id(tx, &id)?.is_starting() {
            return Ok(());                            // a stale gate read cannot undo accepted work (AC-157)
        }
        let now = runs::change(tx, &id, |r| r.state = Phase::Asking)?;
        move_task(tx, &now, "WAIT FOR HUMAN", "Answer in its window")
    }).await;
    if moved.is_ok() {
        super::told_all(app, r);
    }
}

/// The prompt goes in: `typed` first, so no restart between can type it twice; a miss puts it back.
async fn type_prompt(app: &Shared, r: &Run, server: &Server, pane: &str, prompt: &str) -> Result<(), gate::Miss> {
    let _held = super::Held::take(app, |d| &d.pressing, &format!("run:{}", r.id))
        .ok_or_else(|| gate::Miss::Other("A Run is already being prepared.".to_string()))?;
    let (id, was) = (r.id.clone(), r.state);
    let session = if r.kind == "codex" { None } else {
        app.hooks.session(&server.key(pane)).map(|(_, s)| s).filter(|s| hooks::good_id(s))
    };
    let wrote = store(app, move |tx| runs::change(tx, &id, |now| {
        if now.is_starting() {
            now.state = Phase::Typed;
        }
        if let Some(s) = session {
            now.agent_session.get_or_insert(s);
        }
    })).await;
    if let Err(e) = wrote {
        return Err(gate::Miss::Other(e));                  // what it could not write it does not do (AC-131)
    }
    let sent = gate::deliver(app, r, server, pane, prompt, true).await;
    if sent.is_err() {
        let id = r.id.clone();
        let _ = store(app, move |tx| runs::change(tx, &id, |now| if now.state == Phase::Typed { now.state = was; })).await;
    } else {
        let id = r.id.clone();
        if let Err(why) = store(app, move |tx| super::recent::delivered(tx, &id)).await {
            tracing::warn!(event = "dispatcher.start", error = %why);
        }
    }
    sent
}

async fn watching(app: &Shared, id: &str) {
    let began = Instant::now();
    let (mut typed_at, mut empty_since, mut miss): (Option<Instant>, Option<Instant>, Option<String>) = (None, None, None);
    let box_within = |miss: &Option<String>| match miss { Some(m) => format!("its input box did not come in 60 s: {m}"), None => "its input box did not come in 60 s".to_string() };
    loop {
        let wanted = id.to_string();
        let Ok(r) = store(app, move |tx| runs::by_id(tx, &wanted)).await else { return };
        if !r.is_starting() {
            return;
        }
        let Some((server, pane)) = r.place(app) else {
            // no pane yet: the press that made it is still opening its window, and a walk may have come
            // first; one that never gets it (a restart on the way) fails when a box would have come
            if r.pane_key.is_none() && began.elapsed() < start_scaled(BOX_WITHIN) {
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            return did_not_start(app, &r, None, StartMiss::Other("its window is gone")).await;
        };
        if let Some(code) = exit_of(&server, &pane).await {
            return did_not_start(app, &r, Some((&server, &pane)), StartMiss::Exit(code)).await;
        }
        if r.state == Phase::Typed {
            let at = *typed_at.get_or_insert_with(Instant::now);
            if at.elapsed() >= start_scaled(TAKEN_WITHIN) {
                let why = if r.kind == "codex" { "the prompt was not taken in 30 s; hooks not trusted: open /hooks in Codex, or Codex runs in a shared app-server (AC-156)" } else { "the prompt was not taken in 30 s" };
                return did_not_start(app, &r, Some((&server, &pane)), StartMiss::Other(why)).await;
            }
        } else {
            match gate::read(&server, &pane, &r.kind).await.unwrap_or(Screen::Unknown) {
                Screen::Asks if r.state != Phase::Asking => now_asking(app, &r).await,
                Screen::Asks => {}
                Screen::Empty if r.ad_hoc() => {
                    // + Session's: its box stands, nothing is typed: the human's from here (AC-142)
                    let id = r.id.clone();
                    let _ = store(app, move |tx| runs::change(tx, &id, |now| if now.is_starting() { now.state = Phase::Working; }).map(drop)).await;
                    return;
                }
                Screen::Empty => {
                    if ready(app, &server, &pane, &mut empty_since) && let Some(prompt) = &r.prompt
                        && let Err(m) = type_prompt(app, &r, &server, &pane, prompt).await {
                        miss = Some(m.to_string());
                        if r.state == Phase::Starting && began.elapsed() >= start_scaled(BOX_WITHIN) {
                            return did_not_start(app, &r, Some((&server, &pane)), StartMiss::Other(&box_within(&miss))).await;
                        }
                    }
                }
                _ => {
                    empty_since = None;
                    if r.state == Phase::Starting && began.elapsed() >= start_scaled(BOX_WITHIN) {
                        return did_not_start(app, &r, Some((&server, &pane)), StartMiss::Other(&box_within(&miss))).await;
                    }
                }
            }
        }
        // a question nobody answers may stand all night: no need to look five times a second
        tokio::time::sleep(Duration::from_millis(if r.state == Phase::Asking { 1000 } else { 200 })).await;
    }
}

/// The resume of §9: the run's own command, then the kind's words; its ID's shape checked.
pub fn resume_command(run: &Run) -> Result<String, String> {
    let id = run.agent_session.as_deref().filter(|id| hooks::good_id(id)).ok_or("no conversation is known")?;
    Ok(format!("{} {}", run.command, resume_words(&run.kind, id)))
}

/// The words after an agent's command that resume conversation `id` (Codex's subcommand, Claude Code's
/// option); `id` "" lists the conversations to pick one.
pub fn resume_words(kind: &str, id: &str) -> String {
    let words = if kind == "codex" { "resume" } else { "--resume" };
    if id.is_empty() { words.to_string() } else { format!("{words} {id}") }
}

/// The agent starts anew in a pane where only its shell remains (AC-122): the old run's mark checked,
/// the new one's set, the exit cleared.
pub async fn respawn(app: &Shared, server: &Arc<Server>, pane: &str, old: &str, new: &str, folder: &str, command: &str) -> Result<(), String> {
    is_marked(server, pane, old).await?;
    mark(server, pane, new).await?;
    if let Some(w) = app.dispatcher.watch.lock().get_mut(&server.key(pane)) {
        w.clear_subagents();                                // a new agent: none of the last one's
    }
    server.run(format!("respawn-pane -k -t {pane} {}", window_command(folder, command, &tmux_of(server), false))).await.map(drop).map_err(|e| format!("tmux did not start it: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::Tmux;

    #[tokio::test]
    async fn ac157_a_stale_start_watch_cannot_overwrite_accepted_work() {
        use crate::tasks::{ops::NewTask, store::HUMAN, tools::{self, Who}};
        let (root, app) = crate::testutil::app_with_tasks("start-stale");
        let key = app.tmux.local.key("%1");
        let (task, stale) = app.tasks.with(|tx| {
            let t = ops::create(tx, HUMAN, NewTask { title: "early question".into(), session: Some(key.clone()), ..Default::default() }, 1)?;
            let like = Run { agent_id: "a".into(), agent_name: "Fixer".into(), project: "default".into(), prompt: Some("First prompt".into()), agent_session: Some("c0ffee".into()), ..Default::default() };
            let id = runs::add(tx, &like, &[t.key], Default::default(), 2)?;
            let r = runs::change(tx, &id, |r| { r.state = Phase::Typed; r.pane_key = Some(key.clone()); })?;
            Ok((t.id, r))
        }).unwrap();
        let who = Who { key, agent: "claude".into(), title: "Fixer".into(), project: Some("default".into()), dir: None, supervisor: false };
        tools::run(&app.tasks, &root, &who, "ask_human", &serde_json::json!({"id": task, "questions": crate::tasks::closeout::ASKED})).unwrap();
        let accepted = app.tasks.with(|tx| runs::by_id(tx, &stale.id)).unwrap();
        now_asking(&app, &stale).await;
        did_not_start(&app, &stale, None, StartMiss::Other("the earlier timeout expired")).await;
        app.tasks.with(|tx| {
            assert_eq!(runs::by_id(tx, &stale.id)?, accepted);
            assert_eq!(ops::get(tx, &task)?.status, "WAIT FOR HUMAN");
            Ok(())
        }).unwrap();
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    fn until(mut ok: impl FnMut() -> bool) -> bool {
        let end = Instant::now() + Duration::from_secs(10);
        while Instant::now() < end {
            if ok() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    }

    /// Under the `sh` of Debian, where a Mac has it too: with job control it dies of its job's Ctrl-C.
    #[test]
    fn ac123_a_ctrl_c_under_dash_is_recorded_and_the_window_keeps_a_shell() {
        if !std::path::Path::new("/bin/dash").exists() {
            return;
        }
        let Some(s) = Tmux::start("wrapper-dash", (80, 24), "/bin/sh") else { return };
        let pane = s.read(&["new-window", "-d", "-P", "-F", "#{pane_id}", "/bin/dash", "-c", WRAPPER, "sh", "/", "sleep 300", "tmux"]);
        let pane = pane.trim();
        let shown = |what: &str| s.read(&["display", "-p", "-t", pane, what]).trim().to_string();
        assert!(until(|| shown("#{pane_current_command}") == "sleep"), "{}", shown("#{pane_current_command}"));
        s.tmux(&["send-keys", "-t", pane, "C-c"]);
        assert!(until(|| shown("#{@terminum-exit}") == "130"), "exit {:?}", shown("#{@terminum-exit}"));
        let pid = shown("#{pane_pid}");
        let groups = || String::from_utf8_lossy(&std::process::Command::new("ps").args(["-o", "pgid=,tpgid=", "-p", &pid]).output().unwrap().stdout).split_whitespace().map(str::to_string).collect::<Vec<_>>();
        assert!(until(|| { let g = groups(); g.len() == 2 && g[0] == g[1] }), "its shell does not hold the terminal: {:?}", groups());
    }
}
