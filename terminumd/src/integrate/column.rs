// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The board's INTEGRATION column (`terminum-integration.md` AC-193): per project with integration on, the batch in
//! work as one group headed by its integration task, the queue in its order with each card's place or what it waits
//! for, the queue paused or waiting for another project, tasks returned to work; and its keys: Pause, Resume, a
//! human's Integrate, Batches…. terminumd says each state in words; the page draws them.

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::store::{self as bs, Batch};
use crate::app::Shared;
use crate::tasks::api::{answer, store};
use crate::tasks::integration;
use crate::tasks::integration_queue::{ordinal, queue_of};
use crate::tasks::ops::{self, sql};
use crate::tasks::store::HUMAN;

/// Past batches a page reads at a time.
const BATCHES_PAGE: i64 = 100;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/integration/column", get(column))
        .route("/api/integration/project/{id}/pause", post(pause))
        .route("/api/integration/project/{id}/batches", get(batches))
        .route("/api/integration/task/{id}/queue", post(queue_task))
        .route("/api/integration/task/{id}/out", post(take_out))
}

/// A batch's state in words, as its group's head says it (AC-193).
pub fn state_words(tx: &Transaction, b: &Batch, why: Option<&str>) -> Result<String, String> {
    let round = if b.round > 1 { format!(", round {}", b.round) } else { String::new() };
    Ok(match b.state.as_str() {
        "rebuild" => "building".into(),
        "checking" if b.round > 1 => format!("checking again{round}"),
        "checking" => "checking".into(),
        "green" | "publishing" => "publishing".into(),
        "after" => "After runs".into(),
        "cleanup" => "Cleanup runs".into(),
        "red" => {
            let i = bs::task_of(tx, b)?.unwrap_or_default();
            let status = ops::get(tx, &i).map(|t| t.status).unwrap_or_default();
            if status == "IN PROGRESS" {
                let agent: Option<String> = tx.query_row("SELECT i.agent FROM project_integration i WHERE i.project = ?1", [&b.project], |r| r.get(0)).optional().map_err(sql)?.flatten();
                let name = agent.and_then(|a| tx.query_row("SELECT name FROM agents WHERE id = ?1", [a], |r| r.get::<_, String>(0)).ok()).unwrap_or_else(|| "its agent".into());
                format!("red{round}: {name} analyses it")
            } else {
                "red: waits for you".into()
            }
        }
        "not published" => format!("not published: {}", why.unwrap_or("")),
        "after failed" => format!("After failed: {}", why.unwrap_or("")),
        other => other.to_string(),
    })
}

/// One project's column part.
fn of_project(tx: &Transaction, project: &str) -> Result<Value, String> {
    let (paused, waits): (bool, Option<String>) = tx.query_row("SELECT paused, waits_why FROM project_integration WHERE project = ?1", [project], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional().map_err(sql)?.unwrap_or((false, None));
    let id: Option<i64> = tx.query_row("SELECT id FROM integration_batches WHERE project = ?1 AND state NOT IN ('ended', 'given up') ORDER BY id DESC LIMIT 1", [project], |r| r.get(0))
        .optional().map_err(sql)?;
    let batch = match id {
        Some(id) => {
            let b = bs::by_id(tx, id)?;
            let (why, created): (Option<String>, i64) = tx.query_row("SELECT why, created_at FROM integration_batches WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
            let tasks: Vec<String> = b.tasks.iter().filter_map(|k| bs::slug(tx, k).ok()).collect();
            json!({"id": b.id, "number": b.number, "task": bs::task_of(tx, &b)?, "state": b.state, "words": state_words(tx, &b, why.as_deref())?,
                   "since": b.step_started.unwrap_or(created), "tasks": tasks, "round": b.round, "pane": b.pane})
        }
        None => Value::Null,
    };
    let queue: Vec<Value> = queue_of(tx, project)?.into_iter().filter(|e| e.batch.is_none()).enumerate().map(|(i, e)| {
        let (stage, waits): (Option<String>, Option<String>) = tx.query_row("SELECT stage, waits_for FROM integration_entries WHERE task = ?1", [&e.task], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional().map_err(sql)?.unwrap_or((None, None));
        Ok(json!({"id": e.id, "place": ordinal(i + 1), "verifying": stage.is_some(), "waits_for": waits.filter(|w| !w.is_empty())}))
    }).collect::<Result<_, String>>()?;
    // tasks back in work from the queue, until queued again or out of IN PROGRESS
    let mut returned = serde_json::Map::new();
    for (key, id) in ops::rows(tx, "SELECT id, slug FROM tasks WHERE project = ?1 AND status = 'IN PROGRESS' AND archived_at IS NULL", [project], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        match ops::how_left(tx, &key)?.as_deref() {
            Some("conflict") => { returned.insert(id, json!("returned: conflict")); }
            Some("red") => { returned.insert(id, json!("returned: red check")); }
            _ => {}
        }
    }
    let paused_by = if paused { Some(format!("Paused by {}", ops::user_name(tx, HUMAN)?)) } else { None };
    Ok(json!({"project": project, "paused": paused, "paused_by": paused_by, "waits": waits, "batch": batch, "queue": queue, "returned": returned}))
}

#[derive(serde::Deserialize)]
struct Shown {
    #[serde(default)]
    projects: String,
}

/// The column's parts for the projects shown (none named: every project with integration on).
async fn column(State(app): State<Shared>, Query(q): Query<Shown>) -> Response {
    let mut shown = store(&app, move |tx| {
        let on = integration::on_projects(tx)?;
        let wanted: Vec<&str> = q.projects.split(',').filter(|p| !p.is_empty()).collect();
        let mut parts = Vec::new();
        let mut ids: Vec<&String> = on.iter().filter(|p| wanted.is_empty() || wanted.contains(&p.as_str())).collect();
        ids.sort();
        for p in ids {
            parts.push(of_project(tx, p)?);
        }
        Ok(json!({"projects": parts}))
    }).await;
    // a step's window is a session of terminumd's own tmux: its key, as a session's, for the group's head to show it
    if let Ok(v) = &mut shown {
        for part in v["projects"].as_array_mut().into_iter().flatten() {
            if let Some(pane) = part["batch"]["pane"].as_str().map(str::to_string) {
                part["batch"]["window"] = json!(app.tmux.local.key(&pane));
            }
        }
    }
    answer(shown)
}

/// Pause or Resume (AC-193): a paused queue starts no batch; the one in work goes on.
async fn pause(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Value>) -> Response {
    let paused = body["paused"].as_bool().unwrap_or(true);
    let done = store(&app, move |tx| {
        ops::check_project(tx, &project)?;
        let n = tx.execute("UPDATE project_integration SET paused = ?2 WHERE project = ?1 AND is_on = 1", (&project, paused)).map_err(sql)?;
        if n == 0 {
            return Err("Integration is off for this project.".into());
        }
        Ok(json!({"ok": true, "paused": paused}))
    }).await;
    if done.is_ok() {
        crate::page::projects::tell(&app);
    }
    answer(done)
}

#[derive(serde::Deserialize)]
struct Before {
    before: Option<i64>,
}

/// A project's batches, newest first, 100 at a time (AC-193), with how many of the last 20 were green.
async fn batches(State(app): State<Shared>, Path(project): Path<String>, Query(q): Query<Before>) -> Response {
    answer(store(&app, move |tx| batches_of(tx, &project, q.before)).await)
}

fn batches_of(tx: &Transaction, project: &str, before: Option<i64>) -> Result<Value, String> {
        ops::check_project(tx, project)?;
        let rows: Vec<(i64, String, String, Option<String>, Option<String>, String, Option<String>, i64)> = ops::rows(tx,
            "SELECT id, tasks, base_sha, candidate_sha, task, state, why, round FROM integration_batches WHERE project = ?1 AND number < ?2 ORDER BY number DESC LIMIT ?3",
            rusqlite::params![project, before.unwrap_or(i64::MAX), BATCHES_PAGE], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)))?;
        let mut out = Vec::new();
        for (id, _, base, candidate, task, state, why, round) in rows {
            let b = bs::by_id(tx, id)?;
            let tasks: Vec<String> = b.tasks.iter().filter_map(|k| bs::slug(tx, k).ok()).collect();
            let checked: Option<i64> = tx.query_row("SELECT sum(ended - started) FROM integration_rounds WHERE batch = ?1 AND ended IS NOT NULL AND started IS NOT NULL", [id], |r| r.get(0)).map_err(sql)?;
            let result = match state.as_str() {
                "ended" if round > 1 => format!("green in round {round}"),
                "ended" => "green".into(),
                "given up" => format!("nothing published: {}", why.unwrap_or_default()),
                other => other.to_string(),
            };
            out.push(json!({"number": b.number, "tasks": tasks, "base": super::git::short(&base), "candidate": candidate.as_deref().map(super::git::short),
                            "task": task.map(|k| bs::slug(tx, &k)).transpose()?, "rounds": round, "result": result, "checked_seconds": checked,
                            "log": format!("batch-{}.{round}.check.log", b.number)}));
        }
        let green: i64 = tx.query_row("SELECT count(*) FROM (SELECT state FROM integration_batches WHERE project = ?1 ORDER BY number DESC LIMIT 20) WHERE state = 'ended'", [project], |r| r.get(0)).map_err(sql)?;
        let last: i64 = tx.query_row("SELECT min(20, count(*)) FROM integration_batches WHERE project = ?1", [project], |r| r.get(0)).map_err(sql)?;
        Ok(json!({"batches": out, "green": green, "of": last, "more": out.len() as i64 == BATCHES_PAGE}))
}

/// A human's Integrate on an IN PROGRESS card, or its drop on INTEGRATION (AC-193): its session's folder's HEAD.
async fn queue_task(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let wanted = id.clone();
    let session = match store(&app, move |tx| Ok(ops::get(tx, &wanted)?.session)).await {
        Ok(s) => s,
        Err(e) => return answer::<Value>(Err(e)),
    };
    // where its agent works, as its hooks said, else its session's folder (Page never guesses a path: terminumd's)
    let folder = session.and_then(|key| {
        let layout = app.layout.read();
        crate::page::marks::find_item(&layout, &key).and_then(|row| row["agentDir"].as_str().or(row["dir"].as_str()).filter(|d| !d.is_empty()).map(str::to_string))
    });
    let (task, tasks) = (id.clone(), app.tasks.clone());
    let done = tokio::task::spawn_blocking(move || crate::tasks::integration_queue::integrate_by_human(&tasks, &task, folder.as_deref()))
        .await.map_err(|e| e.to_string()).and_then(|r| r);
    if done.is_ok() {
        crate::tasks::api::told(&app, &id);
        crate::page::projects::tell(&app);
    }
    answer(done.map(|said| json!({"ok": true, "said": said})))
}

/// A human's Take out (AC-187, AC-193): the task IN PROGRESS again, out of the queue and any batch, which goes on
/// without it.
pub(crate) fn out(tx: &Transaction, id: &str, at: i64) -> Result<(), String> {
    let t = ops::get(tx, id)?;
    if t.status != crate::tasks::ops::INTEGRATION {
        return Err(format!("{id} is not queued."));
    }
    if bs::by_task(tx, &t.key)?.is_some() {
        return Err(format!("{id} is a batch's integration task: it ends with its batch."));
    }
    ops::place(tx, HUMAN, &t, "IN PROGRESS", at)?;
    ops::log(tx, &t.key, HUMAN, "queue", &format!("{} took it out of the integration queue.", ops::user_name(tx, HUMAN)?), None, at).map(drop)
}

async fn take_out(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let task = id.clone();
    let done = store(&app, move |tx| out(tx, &task, ops::now())).await;
    if done.is_ok() {
        crate::tasks::api::told(&app, &id);
        crate::page::projects::tell(&app);
    }
    answer(done.map(|()| json!({"ok": true})))
}

#[cfg(test)]
#[path = "column_tests.rs"]
mod tests;
