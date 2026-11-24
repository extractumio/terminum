// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-245: the signed-in owner and the server-identified Operator share the same hold transitions.

use axum::{Router, extract::{Path, State}, response::Response, routing::{get, post}};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::app::Shared;
use crate::dispatcher::{api::resume_hold, letgo};
use crate::roles::supervisor as authority;
use crate::tasks::{api::{answer, store, told}, ops::now, store::HUMAN};
use super::holds::{self, Applied, Disposition, Hold, Kind, Request};

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-projects/{project}/supervisor/holds", get(list))
        .route("/api/task-projects/{project}/supervisor/holds/{id}/release", post(owner_release))
}

async fn list(State(app): State<Shared>, Path(project): Path<String>) -> Response {
    answer(crate::http::off(move || app.tasks.snapshot(super::SCHEMA, |tx| {
        crate::tasks::ops::check_project(tx, &project)?;
        holds::list(tx, &project)
    })).await)
}

pub(super) fn heard(app: &Shared, applied: Applied) {
    nudged(app, &applied.nudges);
    if let Some(run) = applied.run {
        letgo::let_go_of(app, *run);
    } else if let Some(task) = applied.task {
        told(app, &task);
    }
    changed(app);
}

/// Supervisor comments reach their tasks' linked agents as any comment does (AC-241, terminum-dispatcher.md AC-209).
pub(crate) fn nudged(app: &Shared, nudges: &[(String, Option<String>)]) {
    for (task, event) in nudges {
        if let Some(event) = event { crate::tasks::notify::comment_added(app, task, event, None); }
        told(app, task);
    }
}

/// An interim verification hold began or ended on this task (AC-233).
pub(crate) fn interim_changed(app: &Shared, task: &str) {
    told(app, task);
    changed(app);
}

fn changed(app: &Shared) {
    crate::page::projects::tell(app);
    app.dispatcher.soon();
    app.supervisor.incident_changed();
}

async fn owner_release(State(app): State<Shared>, Path((project, id)): Path<(String, i64)>) -> Response {
    answer(release(&app, project, id, None, None).await)
}

pub(super) async fn release(app: &Shared, project: String, id: i64, pane: Option<String>, incident: Option<i64>) -> Result<Value, String> {
    let scope = project.clone();
    let key = pane.clone();
    let held = store(app, move |tx| {
        if let Some(key) = &key { authority::require(tx, key, &scope, "release_hold")?; }
        let hold = holds::get(tx, &scope, id)?;
        if incident.is_some() && hold.incident != incident {
            return Err("That hold is not on this incident.".into());
        }
        if key.is_some() && hold.kind == Kind::StopRun
            && let Some(card) = super::action_tasks::diverted_hold_run(tx, &scope, id, now())?
        {
            return Ok(Err(card.task));
        }
        if hold.kind != Kind::StopRun {
            let actor = key.as_ref().map_or_else(|| Ok(HUMAN.into()), |_| authority::author(tx, &scope))?;
            holds::release(tx, &scope, &hold, &actor, now())?;
        }
        Ok(Ok(hold))
    }).await?;
    let held = match held {
        Ok(hold) => hold,
        Err(card) => { told(app, &card); return Ok(json!({"decision_cards": [card]})); }
    };
    if held.kind == Kind::StopRun
        && let Some(cards) = resume(app, project, pane, &held).await?
    {
        return Ok(cards);
    }
    if let Some(task) = &held.task { told(app, task); }
    changed(app);
    Ok(json!({"id": id, "released": true}))
}

async fn resume(app: &Shared, project: String, pane: Option<String>, hold: &Hold) -> Result<Option<Value>, String> {
    let task = hold.task.clone().ok_or("The held task is gone.")?;
    let answer = resume_hold(app, project, hold.id, pane, task).await?;
    if answer.get("decision_cards").is_some() { return Ok(Some(answer)); }
    if answer.get("run").is_none() {
        return Err(answer["full"]["words"].as_str().unwrap_or("The run could not be resumed.").into());
    }
    Ok(None)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReleaseArgs { id: i64 }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopArgs { id: String, signal: String, reason: String }

pub(crate) async fn tool(app: &Shared, key: String, project: String, tool: String, args: Value) -> Result<String, String> {
    if tool == "release_hold" {
        let input: ReleaseArgs = serde_json::from_value(args).map_err(|e| format!("Release hold: {e}"))?;
        return release(app, project, input.id, Some(key), None).await.map(|r| r.to_string());
    }
    let request = if tool == "stop_task" {
        let input: StopArgs = serde_json::from_value(args).map_err(|e| format!("Stop task: {e}"))?;
        Request { kind: Kind::StopRun, signal: input.signal, task: Some(input.id), reason: input.reason }
    } else {
        serde_json::from_value(args).map_err(|e| format!("Hold: {e}"))?
    };
    let done = store(app, move |tx| {
        authority::require(tx, &key, &project, &tool)?;
        let actor = authority::author(tx, &project)?;
        holds::apply(tx, &project, request, &actor, now())
    }).await?;
    match done {
        Disposition::Applied(done) => {
            let result = json!({"id": done.id, "shadow": done.shadow});
            if !done.shadow { heard(app, done); }
            Ok(result.to_string())
        }
        Disposition::Card(card) => {
            told(app, &card.task);
            Ok(json!({"decision_card": card.task}).to_string())
        }
    }
}
