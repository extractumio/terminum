// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Letting go of what Dispatcher holds (`terminum-dispatcher.md` AC-138, AC-142): a human's, or the project's
//! manager's by its stop_task (`terminum-agents.md` AC-175); and a budget changed on a running task (AC-126).

use axum::extract::{Path, State};
use axum::response::Response;
use serde_json::{json, Value};

use super::press::budget_told;
use super::runs::{self, Run};
use super::{acts, store};
use crate::app::Shared;
use crate::tasks::agents;
use crate::tasks::api::{answer, told};
use crate::tasks::ops::{self, Task};

/// The human lets go of a session Dispatcher holds: its open tasks fail, its slot frees, its window stays (AC-142).
pub(super) async fn let_go(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(store(&app, move |tx| runs::let_go(tx, &id)).await.map(|r| let_go_of(&app, r)))
}

/// What a run let go of says, and every page hears of its tasks.
fn let_go_of(app: &Shared, r: Run) -> Value {
    super::told_all(app, &r);
    json!({"ok": true, "ids": r.ids})
}

/// The human gives up on a run they cannot reach or no longer want (AC-138).
pub(super) async fn give_up(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(gave_up(&app, &id, "The human").await)
}

/// `who` gives up on task `id`: a wait is called off, the task where it stood (AC-158); else its run is given up.
/// The tasks it concerned.
pub(crate) async fn gave_up(app: &Shared, id: &str, who: &'static str) -> Result<Value, String> {
    let wanted = id.to_string();
    let done = store(app, move |tx| {
        let key = ops::key_of(tx, &wanted)?;
        let Some(w) = super::waits::of_task(tx, &key)? else { return runs::give_up(tx, &key).map(Some) };
        let name = agents::by_id(tx, &w.agent)?.map_or(w.agent, |a| a.name);
        super::waits::call_off(tx, &key, &format!("{who} called off its wait for {name}.")).map(|()| None)
    }).await?;
    match done {
        None => {
            told(app, id);
            Ok(json!({"ok": true, "ids": [id]}))
        }
        Some(r) => Ok(let_go_of(app, r)),
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
