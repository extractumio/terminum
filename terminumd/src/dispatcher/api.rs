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
use super::runs::{self, How, Phase, Run, Started};
use super::press::{again_words, also_of, answers_of, bring_back, facts, way, ways, with_open, Asked, Facts, Way};
use super::pool;
use super::screen::Screen;
use super::goes_on::{from_shell, resume, went_on};
use super::places::{ad_hoc, launch, no_folder, place};
use super::{acts, start, store};
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
        .route("/api/tasks/{id}/give-up", post(super::letgo::give_up))
        .route("/api/task-projects/{id}/session", post(ad_hoc))
        .route("/api/runs/{id}/let-go", post(super::letgo::let_go))
        .merge(super::queue::routes())
}

/// What RUN's dialog shows (AC-121, AC-138): the tasks' project's pool, each saying what keeps it busy; an agent named that cannot
/// take them is refused here, so the window never offers another under its name. `no_folder`: nothing can start (AC-140).
async fn offer(State(app): State<Shared>, Path(id): Path<String>, axum::extract::Query(asked): axum::extract::Query<Asked>) -> Response {
    answer(store(&app, move |tx| {
        let (tasks, w, moved_from) = ways(tx, &id, &also_of(&asked.also), asked.agent.as_deref(), false)?;
        let w = w?;
        let t = &tasks[0];
        let (name, host, dir) = ops::project_place(tx, &t.project)?;
        let all: Vec<Agent> = agents::of_project(tx, &t.project)?;
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
        Ok(json!({"first": w.first_words(), "agents": offered, "last": last, "dir": dir, "host": host, "project": t.project, "no_folder": lacks,
                  "prompt_rev": shown.rev(), "prompt": shown.shown(), "moved": moved, "by_agent": tasks.iter().any(|t| t.by_agent), "ids": tasks.iter().map(|t| &t.id).collect::<Vec<_>>()}))
    }).await)
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub(super) struct Press {
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
    /// the project's manager pressed, by its tool (terminum-agents.md AC-179): the user it writes as. Kept with a
    /// wait so its start says the same; never taken from a page's request
    #[serde(default)]
    manager: Option<String>,
    /// how it was pressed, kept with its run and with a wait (AC-183); a page's request says only a human's way
    #[serde(default)]
    how: How,
}

impl Press {
    /// The words a first run's agent hears, from the tasks as they stand: refused when they are not what the human
    /// saw, as an agent may have changed a task meanwhile (AC-121).
    fn first_words(&self, tx: &rusqlite::Transaction, tasks: &[Task]) -> Result<Facts, String> {
        let mut goes = facts(tx, tasks)?;
        goes.auto = self.auto;
        goes.manager = self.manager.is_some();
        if self.prompt_rev.as_ref().is_some_and(|rev| *rev != goes.rev()) {
            return Err("The task's text changed: look at it again.".to_string());
        }
        Ok(goes)
    }

    /// Who pressed: the manager's user, else the human.
    fn by(&self) -> &str {
        self.manager.as_deref().unwrap_or(HUMAN)
    }

    /// Who asked for the run it starts, and how (AC-183): a manager's press is its own user's.
    pub(super) fn started(&self) -> Started<'_> {
        match &self.manager {
            Some(user) => Started { by: user, how: self.how },
            None => Started::of(self.how),
        }
    }

    /// Keeps the presser's message on each of `tasks`, the first with its files (AC-138, AC-185); a wait kept it when it
    /// began (AC-158), and nothing is kept twice.
    fn keep_on(&self, tx: &rusqlite::Transaction, tasks: &[Task], text: &str, files: &[String]) -> Result<(), String> {
        if self.keep(tx, &tasks[0].id, text, files)?.is_some() {
            for t in &tasks[1..] {
                ops::comment(tx, self.by(), &t.id, text, ops::now())?;
            }
        }
        Ok(())
    }

    /// Keeps the presser's message on `task`: a human's as theirs, with its files; a manager's as its comment.
    fn keep(&self, tx: &rusqlite::Transaction, task: &str, text: &str, files: &[String]) -> Result<Option<String>, String> {
        match &self.manager {
            None => ops::keep_human_comment(tx, task, text, files, ops::now()),
            Some(user) => ops::comment(tx, user, task, text, ops::now()).map(Some),
        }
    }
}

/// What a press that would start a process comes to: its run, a wait for a slot (AC-158), or the queue offered,
/// nothing kept (AC-169).
pub(super) enum Next {
    Run(String),
    Kept(Kept),
}

/// A press that started nothing.
pub(super) enum Kept {
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
    // Auto-Dispatcher's round takes no slot and waits behind none (AC-170)
    if alone && crate::tasks::auto::round_of(tx, &task.key)?.is_some() {
        return Ok(None);
    }
    if let Some(full) = slots.check_for(tx, agent, project, alone.then_some(task.key.as_str()), but)? {
        return Ok(Some(full));
    }
    Ok(super::waits::ahead(tx, &agent.id, project, &task.key)?.map(|first| pool::Full { words: format!("{first} waits for it first."), close: None }))
}

impl Kept {
    /// What it answers; a wait is told to every page.
    pub(super) fn answer(self, app: &Shared, t: &Task) -> Value {
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
pub(super) fn wait_if_full(tx: &rusqlite::Transaction, agent: &Agent, project: &str, (task, alone): (&Task, bool), but: Option<&str>, (body, comment, files): (&Press, Option<&str>, &[String])) -> Result<Option<Kept>, String> {
    let Some(full) = no_room(tx, &pool::Slots::read(tx)?, agent, project, (task, alone), but)? else { return Ok(None) };
    if !body.queue {
        return Ok(Some(Kept::Offered(full)));
    }
    let why = full.words;
    if let Some(c) = comment {
        body.keep(tx, &task.id, c, files)?;
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
    // always a session of its own: a conversation a human's RUN left is never gone on in by its words
    pressed(app, id, Press { agent: Some(agent.to_string()), auto: true, fresh: true, how: How::Auto, ..Default::default() }).await
}

/// The press on RUN: a comment is kept whatever follows (AC-122).
async fn press(State(app): State<Shared>, Path(id): Path<String>, Json(mut body): Json<Press>) -> Response {
    body.manager = None;
    // a page presses for the human alone: it never starts a run as Dispatcher's own (AC-183)
    body.how = body.how.of_page();
    answer(pressed(&app, &id, body).await)
}

/// The manager's press, by its run_task (terminum-agents.md AC-179): as a human's, its words said to be the manager's.
pub(crate) async fn press_by_manager(app: &Shared, id: &str, user: &str, agent: Option<String>, comment: Option<String>, (fresh, queue): (bool, bool)) -> Result<Value, String> {
    pressed(app, id, Press { agent, comment, fresh, queue, manager: Some(user.to_string()), ..Default::default() }).await
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
    let (kept, by) = (body.clone(), body.clone());
    let (tasks, w, again) = store(app, move |tx| {
        let (tasks, w, _) = ways(tx, &wanted, &also, agent.as_deref(), fresh)?;
        // a run that starts keeps its comment with it, in the transaction that adds it; going on, or refused, it stays, once
        if !matches!(w, Ok(Way::First)) && let Some(c) = &said {
            by.keep_on(tx, &tasks, c, &attached)?;
        }
        let again = match &w {
            Ok(Way::Anew(_)) => by.first_words(tx, &tasks)?.prompt(said.as_deref()),
            _ => {
                // its agent asked: what the humans answered since goes, theirs alone (AC-157), and the pressing manager's
                // own message (terminum-agents.md AC-179), never another agent's comment
                let asked = matches!(&w, Ok(Way::Live(r) | Way::Shell(r) | Way::Resume(r)) if r.asked);
                let mut answers = if asked { answers_of(tx, &tasks)? } else { None };
                if let (Some(a), Some(c), true) = (&mut answers, &said, by.manager.is_some()) {
                    a.push(c.clone());
                }
                with_open(again_words(&tasks[0].id, &ops::user_name(tx, by.by())?, said.as_deref(), answers.as_deref(), by.manager.is_some()), &tasks)
            }
        };
        Ok((tasks, w, again))
    }).await?;
    if comment.is_some() && !matches!(w, Ok(Way::First)) {
        told(app, id);
    }
    match w? {
        Way::First => first_run(app, &tasks, body, kept, comment).await,
        Way::Live(r) => {
            let (server, pane) = pane_of(app, &r)?;
            // the walk may not have read yet that its agent ended
            if start::exit_of(&server, &pane).await.is_some() {
                // with no conversation its next press is a first run's, the words shown (AC-122)
                if start::resume_command(&r).is_err() {
                    return Err("Its agent has just ended: run it again.".to_string());
                }
                return from_shell(app, &r, &server, &pane, again, &tasks, kept).await;
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
            went_on(app, &r, &tasks, kept.manager.is_some()).await
        }
        Way::Shell(r) | Way::Anew(r) => {
            let (server, pane) = pane_of(app, &r)?;
            from_shell(app, &r, &server, &pane, again, &tasks, kept).await
        }
        Way::Resume(r) => resume(app, r, &tasks, again, kept).await,
    }
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

async fn first_run(app: &Shared, tasks: &[Task], body: Press, kept: Press, comment: Option<String>) -> Result<Value, String> {
    let t = &tasks[0];
    let (all, said, attached, closing) = (tasks.to_vec(), comment, body.files.clone(), body.close.is_some());
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
        let goes = kept.first_words(tx, &now)?;
        // the one check that counts: in the transaction that takes the slots; a press that closed a holder
        // for its room does not wait (AC-141)
        if closing && let Some(full) = pool::check_for(tx, &ag, &project, (now.len() == 1).then_some(now[0].key.as_str()), None)? {
            return Err(full.words);
        }
        if let Some(kept) = wait_if_full(tx, &ag, &project, (&now[0], now.len() == 1), None, (&kept, said.as_deref(), &attached))? {
            return Ok(Next::Kept(kept));
        }
        let prompt = goes.prompt(said.as_deref());
        if let Some(c) = &said {
            kept.keep_on(tx, &now, c, &attached)?;
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
        runs::add(tx, &Run::of(&ag, host, f, project, Some(prompt)), &keys, kept.started(), ops::now()).map(Next::Run)
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
