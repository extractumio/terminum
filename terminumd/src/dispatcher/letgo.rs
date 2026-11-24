// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Letting go of what Dispatcher holds (`terminum-dispatcher.md` AC-138, AC-142): a human's, or the project's
//! manager's by its stop_task (`terminum-agents.md` AC-179); and a budget changed on a running task (AC-126).

use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::json;

use super::press::budget_told;
use super::runs::{self, Run};
use super::{acts, store};
use crate::app::Shared;
use crate::tasks::agents;
use crate::tasks::api::{answer, told};
use crate::tasks::ops::{self, Task};

/// The human lets go of a session Dispatcher holds: its open tasks fail, its slot frees, its window stays (AC-142).
pub(super) async fn let_go(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(store(&app, move |tx| runs::let_go(tx, &id)).await.map(|r| json!({"ok": true, "ids": let_go_of(&app, r)})))
}

/// Every page hears of the tasks of a run let go of: their IDs.
pub(crate) fn let_go_of(app: &Shared, r: Run) -> Vec<String> {
    if let Some(key) = &r.pane_key {
        app.dispatcher.release_pane(key, &r.id);
    }
    super::told_all(app, &r);
    r.ids
}

/// The human gives up on a run they cannot reach or no longer want (AC-138).
pub(super) async fn give_up(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(gave_up(&app, &id).await.map(|ids| json!({"ok": true, "ids": ids})))
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Giver { Human, Manager }

/// `who` gives up on task `id`: a wait is called off, the task where it stood (AC-158); else its run is given up.
/// The tasks it concerned.
pub(crate) async fn gave_up(app: &Shared, id: &str) -> Result<Vec<String>, String> {
    gave_up_checked(app, id, Giver::Human, |_, _| Ok(()), |tx, ids| {
        for id in ids {
            let key = ops::key_of(tx, id)?;
            ops::log(tx, &key, crate::tasks::store::HUMAN, "why", "The human gave up on its run.", None, ops::now())?;
        }
        Ok(())
    }).await
}

pub(crate) async fn gave_up_checked(
    app: &Shared, id: &str, giver: Giver,
    check: impl FnOnce(&rusqlite::Transaction, &str) -> Result<(), String> + Send + 'static,
    record: impl FnOnce(&rusqlite::Transaction, &[String]) -> Result<(), String> + Send + 'static,
) -> Result<Vec<String>, String> {
    let decided = std::time::Instant::now();
    let (held, expected) = if giver == Giver::Human {
        let wanted = id.to_string();
        let run = store(app, move |tx| {
            let key = ops::key_of(tx, &wanted)?;
            if super::waits::of_task(tx, &key)?.is_some() { return Ok(None); }
            runs::last_of(tx, &key)
        }).await?;
        let expected = run.as_ref().map(|run| (run.id.clone(), run.pane_key.clone()));
        let held = match run.and_then(|run| run.place(app)) {
            Some((server, pane)) => Some(tokio::time::timeout(std::time::Duration::from_secs(5), server.typing(&pane).lock_owned()).await
                .map_err(|_| "A prompt is being delivered in this session. Give up again once it finishes.".to_string())?),
            None => None,
        };
        (held, expected)
    } else { (None, None) };
    let (had_queue, guard_app) = (held.is_some(), app.clone());
    let who = match giver { Giver::Human => "The human", Giver::Manager => "The project's manager" };
    let wanted = id.to_string();
    let done = store(app, move |tx| {
        check(tx, &wanted)?;
        let key = ops::key_of(tx, &wanted)?;
        if let Some(w) = super::waits::of_task(tx, &key)? {
            let name = agents::name_of(tx, &w.agent)?;
            super::waits::call_off(tx, &key, &format!("{who} called off its wait for {name}."))?;
            record(tx, std::slice::from_ref(&wanted))?;
            Ok(None)
        } else {
            if giver == Giver::Human {
                let current = runs::last_of(tx, &key)?;
                if current.as_ref().map(|run| (run.id.clone(), run.pane_key.clone())) != expected
                    || (!had_queue && current.as_ref().and_then(|run| run.place(&guard_app)).is_some())
                {
                    return Err("Its run changed while Give up waited. Read the task and try again.".into());
                }
            }
            let run = runs::give_up(tx, &key)?;
            if giver == Giver::Human {
                super::giveup::record(tx, &run)?;
            }
            record(tx, &run.ids)?;
            Ok(Some(run))
        }
    }).await?;
    drop(held);
    Ok(match done {
        None => {
            told(app, id);
            vec![id.to_string()]
        }
        Some(r) => {
            let ids = let_go_of(app, r.clone());
            if giver == Giver::Human {
                super::giveup::interrupt(app, &r, decided).await;
            }
            ids
        }
    })
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
