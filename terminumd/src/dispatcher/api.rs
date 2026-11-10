// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! RUN's routes (`terminum-dispatcher.md` AC-121, AC-122): what its dialog offers, and the press.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::gate::{self, Miss};
use super::runs::{self, Phase, Run};
use super::press::{again_words, also_of, budget_told, facts, way, ways, Asked, Way};
use super::pool;
use super::screen::Screen;
use super::places::{ad_hoc, folder_there, launch, no_folder, place, project_named, server_of};
use super::{acts, move_one, move_task, procs, say, start, store};
use crate::app::Shared;
use crate::tasks::agents::{self, Agent};
use crate::tasks::api::{answer, told};
use crate::tasks::ops::{self, Task};
use crate::tasks::store::HUMAN;
use crate::termtext::has_control;
use crate::tmux::Server;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/tasks/{id}/run", get(offer).post(press))
        .route("/api/tasks/{id}/give-up", post(give_up))
        .route("/api/task-projects/{id}/session", post(ad_hoc))
        .route("/api/runs/{id}/let-go", post(let_go))
        .merge(super::queue::routes())
}

/// What RUN's dialog shows (AC-121, AC-138): the pool, each saying what keeps it busy; an agent named that cannot
/// take them is refused here, so the window never offers another under its name. `no_folder`: nothing can start (AC-140).
async fn offer(State(app): State<Shared>, Path(id): Path<String>, axum::extract::Query(asked): axum::extract::Query<Asked>) -> Response {
    answer(store(&app, move |tx| {
        let (tasks, w, moved_from) = ways(tx, &id, &also_of(&asked.also), asked.agent.as_deref(), false)?;
        let w = w?;
        let t = &tasks[0];
        let (name, host, dir) = ops::project_place(tx, &t.project)?;
        let all: Vec<Agent> = agents::list(tx)?;
        let lacks = no_folder(&name, &dir).err().filter(|_| matches!(w, Way::First));
        let slots = pool::Slots::read(tx)?;
        let mut offered = Vec::new();
        for a in &all {
            // one with no room is offered all the same, with its queue (AC-169): what the press would find
            let mut v = json!(a);
            v["full"] = json!(no_room(tx, &slots, a, &t.project, (t, tasks.len() == 1), None)?.as_ref().map(queue_offer));
            offered.push(v);
        }
        if asked.agent.as_ref().is_some_and(|named| !all.iter().any(|a| &a.id == named)) && matches!(w, Way::First) {
            return Err("That agent is gone.".to_string());
        }
        // its conversation goes on on another agent: the window names both, Run from scratch stays (AC-138)
        let moved = match (&w, moved_from) {
            (Way::Shell(r) | Way::Resume(r), Some(from)) => Some(json!({"from": from, "to": r.agent_name, "conversation": r.agent_session})),
            _ => None,
        };
        let last = runs::last_agent(tx, &t.project)?.filter(|l| all.iter().any(|a| a.id == *l));
        let shown = facts(tx, &tasks)?;
        Ok(json!({"first": matches!(w, Way::First), "agents": offered, "last": last, "dir": dir, "host": host, "project": t.project, "no_folder": lacks,
                  "prompt_rev": shown.rev(), "prompt": shown.shown(), "moved": moved, "by_agent": tasks.iter().any(|t| t.by_agent), "ids": tasks.iter().map(|t| &t.id).collect::<Vec<_>>()}))
    }).await)
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Press {
    agent: Option<String>,
    comment: Option<String>,
    prompt_rev: Option<String>,
    #[serde(default)]
    files: Vec<String>,
    #[serde(default)]
    fresh: bool,
    /// the idle run whose window this press closes to make room (AC-141)
    close: Option<String>,
    /// the other tasks of this press (AC-138)
    #[serde(default)]
    also: Vec<String>,
    /// the human agreed to its place in the queue, should there be no room (AC-169)
    #[serde(default)]
    queue: bool,
    /// Auto-Dispatcher's press, no human's (AC-170): never read from a request
    #[serde(skip)]
    auto: bool,
}

/// What a press that would start a process comes to: its run, a wait for a slot (AC-158), or the queue offered,
/// nothing kept (AC-169).
enum Next {
    Run(String),
    Kept(Kept),
}

/// A press that started nothing.
enum Kept {
    Waits(String),
    Offered(pool::Full),
}

/// What RUN's window shows of an agent with no room: what holds it, the idle holder a press could close, and the
/// queue's offer (AC-169); the same from its first read and from a press that found no room.
fn queue_offer(full: &pool::Full) -> Value {
    json!({"words": full.words, "close": full.close, "queue": super::waits::offer(&full.words)})
}

/// What stands in the way of a press for `task` on `agent`: a slot's limit, or a wait before it that has room and
/// goes first (AC-158). `alone`: the press runs `task` alone: one of a schedule's may take its kept slot
/// (terminum-schedules.md AC-161).
fn no_room(tx: &rusqlite::Transaction, slots: &pool::Slots, agent: &Agent, project: &str, (task, alone): (&Task, bool), but: Option<&str>) -> Result<Option<pool::Full>, String> {
    if let Some(full) = slots.check_for(tx, agent, project, alone.then_some(task.key.as_str()), but)? {
        return Ok(Some(full));
    }
    Ok(super::waits::ahead(tx, &agent.id, project, &task.key)?.map(|first| pool::Full { words: format!("{first} waits for it first."), close: None }))
}

impl Kept {
    /// What it answers; a wait is told to every page.
    fn answer(self, app: &Shared, t: &Task) -> Value {
        match self {
            Kept::Waits(words) => {
                told(app, &t.id);
                json!({"waits": words})
            }
            Kept::Offered(full) => json!({"full": queue_offer(&full)}),
        }
    }
}

/// A press with no room waits (AC-158): its words, after the human's comment is kept, so it shows while it waits.
/// One that did not agree to the queue is offered it, nothing kept (AC-169).
fn wait_if_full(tx: &rusqlite::Transaction, agent: &Agent, project: &str, (task, alone): (&Task, bool), but: Option<&str>, (body, comment, files): (&Press, Option<&str>, &[String])) -> Result<Option<Kept>, String> {
    let Some(full) = no_room(tx, &pool::Slots::read(tx)?, agent, project, (task, alone), but)? else { return Ok(None) };
    if !body.queue {
        return Ok(Some(Kept::Offered(full)));
    }
    let why = full.words;
    if let Some(c) = comment {
        ops::keep_human_comment(tx, &task.id, c, files, ops::now())?;
    }
    super::waits::add(tx, task, &agent.id, project, &serde_json::to_value(body).map_err(|e| e.to_string())?, ops::now())?;
    Ok(Some(Kept::Waits(super::waits::words(&agent.name, &why))))
}

/// A wait's press, as its route took it, pressed again now (AC-158).
pub(super) async fn press_again(app: &Shared, id: &str, body: Value) -> Result<Value, String> {
    let mut body: Press = serde_json::from_value(body).map_err(|e| format!("its kept press is not readable: {e}"))?;
    body.close = None;
    body.queue = true;                             // a wait, or a schedule's standing press: it waits again
    pressed(app, id, body).await
}

/// Auto-Dispatcher's press (AC-170): a round on the agent that picks, or a task a round chose on its agent; with no
/// room it answers `full` and nothing waits.
pub(super) async fn press_auto(app: &Shared, id: &str, agent: &str) -> Result<Value, String> {
    pressed(app, id, Press { agent: Some(agent.to_string()), auto: true, ..Default::default() }).await
}

/// The press on RUN: a comment is kept whatever follows (AC-122).
async fn press(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Press>) -> Response {
    answer(pressed(&app, &id, body).await)
}

/// Where a run's pane is.
fn pane_of(app: &Shared, r: &Run) -> Result<(Arc<Server>, String), String> {
    r.place(app).ok_or_else(|| "Its session is gone.".to_string())
}

async fn pressed(app: &Shared, id: &str, body: Press) -> Result<Value, String> {
    let comment = body.comment.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(str::to_string);
    if comment.as_deref().is_some_and(has_stray) {
        return Err("The message has control or invisible characters.".to_string());
    }
    let (wanted, said, attached, fresh, also, agent) = (id.to_string(), comment.clone(), body.files.clone(), body.fresh, body.also.clone(), body.agent.clone());
    let kept = body.clone();
    let (tasks, w, who, answers) = store(app, move |tx| {
        let (tasks, w, _) = ways(tx, &wanted, &also, agent.as_deref(), fresh)?;
        // a run that starts keeps its comment with it, in the transaction that adds it; going on, or refused, it stays, once
        if tasks.len() == 1 && !matches!(w, Ok(Way::First)) && let Some(c) = &said {
            ops::keep_human_comment(tx, &wanted, c, &attached, ops::now())?;
        }
        // its agent asked: what the humans answered since goes, theirs alone (AC-157)
        let asked = matches!(&w, Ok(Way::Live(r) | Way::Shell(r) | Way::Resume(r)) if r.asked);
        let answers = if asked && tasks.len() == 1 { ops::answers(tx, &wanted)? } else { None };
        Ok((tasks, w, ops::user_name(tx, HUMAN)?, answers))
    }).await?;
    let t = tasks[0].clone();
    let going_on = !matches!(w, Ok(Way::First));
    if comment.is_some() && going_on {
        told(app, id);
    }
    let again = again_words(&t.id, &who, comment.as_deref(), answers.as_deref());
    match w? {
        Way::First => first_run(app, &tasks, body, kept, comment).await,
        Way::Live(r) => {
            let (server, pane) = pane_of(app, &r)?;
            // the walk may not have read yet that its agent ended
            if start::exit_of(&server, &pane).await.is_some() {
                return from_shell(app, &r, &server, &pane, again, &t, kept).await;
            }
            // its turn is taken for the words about to be typed, so nothing reads it idle meanwhile; no limit stops it:
            // it starts no process (AC-141)
            let (id, asked) = (r.id.clone(), std::time::Instant::now());
            store(app, move |tx| runs::change(tx, &id, |now| now.turn_ended = false).map(drop)).await?;
            if let Err(miss) = gate::deliver(app, &r, &server, &pane, &again, false).await {
                // what it was, unless a human's prompt came meanwhile: its turn is theirs (a Stop says it ended)
                if !app.inputs.human_since(&server.key(&pane), asked) {
                    let (id, was) = (r.id.clone(), r.turn_ended);
                    let _ = store(app, move |tx| runs::change(tx, &id, |now| now.turn_ended = was).map(drop)).await;
                }
                return Err(match miss {
                    Miss::NotEmpty(_) => "The session is not at an empty input box: answer there first".to_string(),
                    Miss::Other(why) => format!("Not delivered: {why}"),
                });
            }
            went_on(app, &r, &t).await
        }
        Way::Shell(r) => {
            let (server, pane) = pane_of(app, &r)?;
            from_shell(app, &r, &server, &pane, again, &t, kept).await
        }
        Way::Resume(r) => resume(app, r, &t, again, kept).await,
    }
}

/// The conversation goes on in a new window (AC-122), on its agent's slot or waiting for one (AC-158).
async fn resume(app: &Shared, r: Run, t: &Task, again: String, kept: Press) -> Result<Value, String> {
    let Ok(line) = start::resume_command(&r) else {
        return could_not_continue(app, &r, "no conversation is known").await;
    };
    if r.host.is_none() && !folder_there(&r.folder).await {
        return could_not_continue(app, &r, "its first folder is gone").await;
    }
    let server = server_of(app, r.host.as_deref())?;
    let new = match next_run(app, &r, t, again, kept).await? {
        Next::Run(new) => new,
        Next::Kept(kept) => return Ok(kept.answer(app, t)),
    };
    let project = project_named(app, &t.project);
    start::open(app, start::Opening { server: &server, project: project.as_ref().map(|(i, n)| (i.as_str(), n.as_str())), folder: &r.folder, command: &line, run: &new, tasks: std::slice::from_ref(&t.id), program: &r.kind, agent: &r.agent_id }).await?;
    start::watch(app, &new);
    Ok(json!({"run": new}))
}

/// What a press types may hold line ends, nothing else unseen: tmux would refuse it after the run was written.
pub(super) fn has_stray(text: &str) -> bool {
    text.split(['\n', '\t']).any(has_control)
}

/// Closes an idle holder for the press that asked: read again here, its screen too: a draft or a question stands in the way (AC-141).
pub(super) async fn close_idle(app: &Shared, run: &str, for_tasks: &str) -> Result<(), String> {
    let at = std::time::Instant::now();
    let wanted = run.to_string();
    let r = store(app, move |tx| runs::by_id(tx, &wanted)).await?;
    let (server, pane) = pane_of(app, &r)?;
    let wanted = r.id.clone();
    let closable = store(app, move |tx| pool::closable(tx, &runs::by_id(tx, &wanted)?)).await?;
    let quiet = closable && app.dispatcher.watch.lock().get(&server.key(&pane)).is_none_or(|w| w.subs.is_empty());
    if !quiet || !matches!(gate::read(&server, &pane, &r.kind).await, Ok(Screen::Empty)) {
        return Err(format!("{}'s session is no longer idle: it stays.", r.names()));
    }
    let stayed = acts::close(app, &r, (&server, &pane), &format!("Session ended: closed for {for_tasks}."), true, at, pool::closable).await?;
    super::told_all(app, &r);
    stayed.map_or(Ok(()), |why| Err(format!("{}'s session stays: {why}.", r.names())))
}

/// What runs in `pane` other than its shell: in front, or stopped or left behind it (AC-122).
async fn what_runs(app: &Shared, server: &Server, pane: &str) -> Result<Option<String>, String> {
    let said = server.run(format!("display -p -t {pane} '#{{pane_current_command}}\t#{{pane_pid}}'")).await.map_err(|e| format!("tmux does not answer: {e}"))?;
    let (name, pid) = said.first().and_then(|l| l.split_once('\t')).unwrap_or(("", ""));
    if !crate::tmux::titles::is_shell(name) {
        return Ok(Some(name.to_string()));
    }
    // the moment after an agent's end its wrapper and a prompt's helpers still run: what stays is a job
    for look in 0..3 {
        let tree = procs::read(app, server).await.ok_or("ps did not answer")?;
        if !procs::under(&tree, pid).any {
            return Ok(None);
        }
        if look < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
    }
    Ok(Some("a job of its shell".to_string()))
}

/// Only its shell remains: the agent starts there anew, by the resume when a conversation is known;
/// the pane's queue held from the look to the start, so no key starts anything between (AC-122).
async fn from_shell(app: &Shared, r: &Run, server: &Arc<Server>, pane: &str, again: String, t: &Task, kept: Press) -> Result<Value, String> {
    let _held = server.typing(pane).lock_owned().await;
    if let Some(program) = what_runs(app, server, pane).await? {
        return Err(format!("{program} runs in its pane."));
    }
    let line = start::resume_command(r).unwrap_or_else(|_| r.command.clone());
    let new = match next_run(app, r, t, again, kept).await? {
        Next::Run(new) => new,
        Next::Kept(kept) => return Ok(kept.answer(app, t)),
    };
    let done = start::respawn(app, server, pane, &r.id, &new, &r.folder, &line).await;
    crate::inputs::wrote(app, crate::inputs::Who::dispatcher("dispatcher.start", &new, &r.ids, &r.agent_id), "launch", server, pane, &line, done.as_ref().err().cloned(), None);
    done?;
    let (a, key, n) = (app.clone(), server.key(pane), new.clone());
    app.dispatcher.panes.lock().insert(key.clone());    // its agent's first hooks
    store(app, move |tx| a.dispatcher.pane_opened(tx, &n, &key).map(drop)).await?;
    start::watch(app, &new);
    Ok(json!({"run": new}))
}

/// The conversation cannot go on: FAILED, its comment says why, and Run from scratch stays (AC-122).
async fn could_not_continue(app: &Shared, r: &Run, why: &str) -> Result<Value, String> {
    let (run, words) = (r.clone(), format!("Conversation {} could not be continued: {why}", r.agent_session.as_deref().unwrap_or("unknown")));
    store(app, move |tx| super::fail(tx, &run, &words)).await?;
    super::told_all(app, r);
    Err(format!("{why}: use Run from scratch."))
}

/// RUN goes on in its live session: every clock starts anew; a finished task reopens alone, an open one
/// with the others waiting with it.
async fn went_on(app: &Shared, r: &Run, t: &Task) -> Result<Value, String> {
    let (id, task, key, status, words) = (r.id.clone(), t.id.clone(), t.key.clone(), t.status.clone(), "The human ran it again.");
    let done = store(app, move |tx| {
        let now = runs::change(tx, &id, |now| { now.state = Phase::Working; now.new_work(); })?;
        bring_back(tx, &task)?;
        if super::plan::finished(&status) {
            move_one(tx, &task, "IN PROGRESS", words)?;
            runs::reopened(tx, &key)?;
        } else {
            move_task(tx, &now, "IN PROGRESS", words)?;
        }
        Ok(now.id)
    }).await?;
    super::told_all(app, r);
    Ok(json!({"run": done}))
}

/// New work on an archived task returns it to the board (AC-122).
fn bring_back(tx: &rusqlite::Transaction, id: &str) -> Result<(), String> {
    if ops::get(tx, id)?.archived_at.is_some() {
        ops::archive(tx, HUMAN, id, false, ops::now())?;
    }
    Ok(())
}

/// A new run of `t` going on from `r`, which is over: its agent, unless another run took it meanwhile. What
/// else was in `r` lets go of its session: it stays open to its own Run, and no agent there holds it.
async fn next_run(app: &Shared, r: &Run, t: &Task, prompt: String, kept: Press) -> Result<Next, String> {
    let (old, key, id, task) = (r.clone(), t.key.clone(), t.id.clone(), t.clone());
    store(app, move |tx| {
        // an agent deleted since has no slot to take: its run's own settings start, or nothing waits for it
        let agent = agents::by_id(tx, &old.agent_id)?;
        match &agent {
            Some(a) => if let Some(kept) = wait_if_full(tx, a, &old.project, (&task, true), Some(&old.id), (&kept, None, &[]))? {
                return Ok(Next::Kept(kept));
            },
            None => if let Some(full) = pool::check(tx, None, &old.project, Some(&old.id))? {
                return Err(full.words);
            },
        }
        if agent.is_none() {
            say(tx, &old, &format!("{} is no longer in the pool: this goes on with the settings it started with.", old.agent_name))?;
        }
        runs::change(tx, &old.id, |was| { was.state = Phase::Closed; was.pane_key = None; })?;
        for other in old.ids.iter().filter(|other| **other != id) {
            ops::link(tx, super::DISPATCHER, other, None, ops::now())?;
        }
        bring_back(tx, &id)?;
        super::waits::remove(tx, &key)?;
        let (from, id) = (old.id.clone(), runs::add(tx, &Run { prompt: Some(prompt), ..old }, &[key], ops::now())?);
        super::usage::carry(tx, &from, &id)?;
        Ok(Next::Run(id))
    }).await
}

/// The human lets go of a session Dispatcher holds: its open tasks fail, its slot frees, its window stays (AC-142).
async fn let_go(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let_go_of(&app, store(&app, move |tx| runs::let_go(tx, &id)).await)
}

/// What a run let go of says, and every page hears of its tasks.
fn let_go_of(app: &Shared, done: Result<Run, String>) -> Response {
    if let Ok(r) = &done {
        super::told_all(app, r);
    }
    answer(done.map(|r| json!({"ok": true, "ids": r.ids})))
}

async fn first_run(app: &Shared, tasks: &[Task], body: Press, kept: Press, comment: Option<String>) -> Result<Value, String> {
    let t = &tasks[0];
    let (all, seen, said, attached, closing) = (tasks.to_vec(), body.prompt_rev.clone(), comment, body.files.clone(), body.close.is_some());
    let (a, host, dir, server) = place(app, &t.project, body.agent.clone()).await?;
    if let Some(holder) = &body.close {
        close_idle(app, holder, &tasks.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join(", ")).await?;
    }
    let (ag, f, project) = (a.clone(), dir.clone(), t.project.clone());
    let next = store(app, move |tx| {
        let now: Vec<Task> = all.iter().map(|t| ops::get(tx, &t.id)).collect::<Result<_, _>>()?;
        for t in &now {
            way(t, runs::last_of(tx, &t.key)?, true)?;
        }
        // what the human saw is what goes: an agent may have changed a task meanwhile
        let mut goes = facts(tx, &now)?;
        goes.auto = kept.auto;
        if seen.is_some_and(|rev| rev != goes.rev()) {
            return Err("The task's text changed: look at it again.".to_string());
        }
        // the one check that counts: in the transaction that takes the slots; a press that closed a holder
        // for its room does not wait (AC-141)
        if closing && let Some(full) = pool::check_for(tx, &ag, &project, (now.len() == 1).then_some(now[0].key.as_str()), None)? {
            return Err(full.words);
        }
        // a round's session takes no slot and waits behind none (AC-170)
        let round = kept.auto && crate::tasks::auto::round_of(tx, &now[0].key)?.is_some();
        if !round && let Some(kept) = wait_if_full(tx, &ag, &project, (&now[0], now.len() == 1), None, (&kept, said.as_deref(), &attached))? {
            return Ok(Next::Kept(kept));
        }
        let prompt = goes.prompt(said.as_deref());
        // a wait kept its comment when it began (AC-158)
        if let Some(c) = &said && ops::keep_human_comment(tx, &now[0].id, c, &attached, ops::now())?.is_some() {
            for t in &now[1..] {
                ops::comment(tx, HUMAN, &t.id, c, ops::now())?;
            }
        }
        for t in &now {
            bring_back(tx, &t.id)?;
        }
        // one live run a task: an older one's window stays the human's, untouched (Run from scratch)
        let keys: Vec<String> = now.iter().map(|t| t.key.clone()).collect();
        for t in &now {
            if let Some(old) = runs::last_of(tx, &t.key)?.filter(|r| !r.is_over() && r.pane_key == t.session) {
                runs::change(tx, &old.id, |r| r.state = Phase::Left)?;
            }
        }
        super::waits::remove(tx, &keys[0])?;
        runs::add(tx, &Run::of(&ag, host, f, project, Some(prompt)), &keys, ops::now()).map(Next::Run)
    }).await?;
    let run = match next {
        Next::Run(run) => run,
        Next::Kept(kept) => return Ok(kept.answer(app, t)),
    };
    let ids: Vec<String> = tasks.iter().map(|t| t.id.clone()).collect();
    launch(app, &run, &server, &t.project, &dir, &a, &ids).await?;
    for id in &ids {
        told(app, id);
    }
    Ok(json!({"run": run}))
}

/// The human gives up on a run they cannot reach or no longer want (AC-138).
async fn give_up(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    // a wait is called off, the task where it stood (AC-158); else its run is given up
    let wanted = id.clone();
    let done = store(&app, move |tx| {
        let key = ops::key_of(tx, &wanted)?;
        let Some(w) = super::waits::of_task(tx, &key)? else { return runs::give_up(tx, &key).map(Some) };
        let name = agents::by_id(tx, &w.agent)?.map_or(w.agent, |a| a.name);
        super::waits::call_off(tx, &key, &format!("The human called off its wait for {name}.")).map(|()| None)
    }).await;
    match done {
        Ok(None) => {
            told(&app, &id);
            answer(Ok(json!({"ok": true, "ids": [id]})))
        }
        given_up => let_go_of(&app, given_up.map(|r| r.unwrap_or_default())),
    }
}

/// A human changed the budget on a running task: it holds at once, and the agent is told as the last
/// warning is (AC-126).
pub fn budget_changed(app: &Shared, task: &Task) {
    let Some(key) = task.session.clone().filter(|k| app.dispatcher.has_run(k)) else { return };
    let (app, id, own) = (app.clone(), task.id.clone(), task.budget_seconds);
    tokio::spawn(async move {
        let k = key.clone();
        let Ok(Some(r)) = store(&app, move |tx| {
            let Some(r) = runs::in_pane(tx, &k)? else { return Ok(None) };
            runs::change(tx, &r.id, |now| now.warned = false).map(Some)
        }).await else { return };
        if let Some((server, pane)) = app.tmux.resolve(&key) {
            let words = budget_told(&id, own, &r);
            acts::tell(&app, &r, &server, &pane, words, "The new budget").await;
        }
    });
}
