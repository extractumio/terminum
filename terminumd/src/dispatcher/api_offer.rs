// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-121: the available pool, queue and prompt preview for the signed-in Run dialog.

use axum::{extract::{Path, Query, State}, response::Response};
use serde_json::{Value, json};

use super::{Kept, Press, Shared, Task, Way, answer, facts, linked_panes, pool, runs, store, told, ways};
use super::super::{places::no_folder, press::{also_of, Asked}, waits};
use crate::tasks::{agents::{self, Agent}, ops};

/// What RUN's dialog shows (AC-121, AC-138): the tasks' project's pool, each saying what keeps it busy; an agent named that cannot
/// take them is refused here, so the window never offers another under its name. `no_folder`: nothing can start (AC-140).
pub(super) async fn offer(State(app): State<Shared>, Path(id): Path<String>, axum::extract::Query(asked): Query<Asked>) -> Response {
    let pane = linked_panes(&app, &id, &also_of(&asked.also)).await;
    answer(store(&app, move |tx| {
        let (tasks, w, moved_from) = ways(tx, &id, &also_of(&asked.also), asked.agent.as_deref(), false, &pane)?;
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
        let shown = facts(tx, &tasks, None)?;
        Ok(json!({"first": w.first_words(), "agents": offered, "last": last, "dir": dir, "host": host, "project": t.project, "no_folder": lacks,
                  "prompt_rev": shown.rev(), "prompt": shown.shown(), "moved": moved, "by_agent": tasks.iter().any(|t| t.by_agent), "ids": tasks.iter().map(|t| &t.id).collect::<Vec<_>>()}))
    }).await)
}

/// What RUN's window shows of an agent with no room: what holds it, the idle holder a press could close, and the
/// queue's offer (AC-169); the same from its first read and from a press that found no room.
fn queue_offer(full: &pool::Full) -> Value {
    json!({"words": full.words, "close": full.close, "queue": waits::offer(&full.words)})
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
    Ok(waits::ahead(tx, &agent.id, project, &task.key)?.map(|first| pool::Full { words: format!("{first} waits for it first."), close: None }))
}

impl Kept {
    /// What it answers; a wait is told to every page.
    pub(in crate::dispatcher) fn answer(self, app: &Shared, t: &Task) -> Value {
        match self {
            Kept::Waits(words) => {
                told(app, &t.id);
                json!({"waits": words})
            }
            Kept::Offered(full) => json!({"full": queue_offer(&full)}),
            Kept::DecisionCards(cards) => super::supervisor::card_answer(app, cards),
        }
    }
}

/// A press with no room waits (AC-158): its words, after the human's comment is kept, so it shows while it waits.
/// One that did not agree to the queue is offered it, nothing kept (AC-169).
pub(in crate::dispatcher) fn wait_if_full(tx: &rusqlite::Transaction, agent: &Agent, project: &str, (task, alone): (&Task, bool), but: Option<&str>, (body, comment, files): (&Press, Option<&str>, &[String])) -> Result<Option<Kept>, String> {
    let Some(full) = no_room(tx, &pool::Slots::read(tx)?, agent, project, (task, alone), but)? else { return Ok(None) };
    if !body.queue {
        return Ok(Some(Kept::Offered(full)));
    }
    let why = full.words;
    let mut body = body.clone();
    if let Some(c) = comment && !body.waited {
        body.kept_comments.push(body.keep(tx, &task.id, c, files)?.0);
    }
    waits::add(tx, task, &agent.id, project, &serde_json::to_value(body).map_err(|e| e.to_string())?, ops::now())?;
    Ok(Some(Kept::Waits(waits::words(&agent.name, &why))))
}
