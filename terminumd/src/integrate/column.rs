// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The board's INTEGRATION column (`terminum-integration.md` AC-193): what terminumd says on each card (the batch in
//! work on its integration task's card, "in batch 7", a place in the queue or what it waits for, a return on a card
//! in IN PROGRESS), each project's queue paused or waiting, and the keys: Pause and Resume, a human's Integrate and
//! Take out, Batches…, and on a deciding batch's cards its keys (`keys.rs`). The column's order is the queue's (`tasks::query`); terminumd says each state in words.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::Transaction;
use serde_json::{json, Value};

use super::git::short;
use super::keys::keys_of;
use super::store::{self as bs, Batch};
use crate::app::Shared;
use crate::tasks::api::{answer, store};
use crate::tasks::integration;
use crate::tasks::integration_queue::ordinal;
use crate::tasks::ops::{self, sql, Task, INTEGRATION};
use crate::tasks::slugs::is_integration;
use crate::tasks::store::HUMAN;

/// Past batches a page reads at a time.
const BATCHES_PAGE: i64 = 100;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/integration/project/{id}/pause", post(pause))
        .route("/api/integration/project/{id}/batches", get(batches))
        .route("/api/integration/task/{id}/queue", post(queue_task))
        .route("/api/integration/task/{id}/out", post(take_out))
}

/// A batch's state in words, as its integration task's card says it (AC-193).
pub fn state_words(tx: &Transaction, b: &Batch) -> Result<String, String> {
    let round = if b.round > 1 { format!(", round {}", b.round) } else { String::new() };
    Ok(match b.state.as_str() {
        "rebuild" => "building".into(),
        "checking" if b.round > 1 => format!("checking again{round}"),
        "checking" => "checking".into(),
        "green" | "publishing" => "publishing".into(),
        bs::PUBLISHED => "published: After waits for its turn".into(),
        "after" => "After runs".into(),
        "cleanup" => "Cleanup runs".into(),
        "red" => {
            let i = bs::task_of(tx, b)?.unwrap_or_default();
            if ops::get(tx, &i).map(|t| t.status).unwrap_or_default() == "IN PROGRESS" {
                let name = integration::read(tx, &b.project)?.agent.map(|a| crate::tasks::agents::name_of(tx, &a)).transpose()?.unwrap_or_else(|| "its agent".into());
                format!("red{round}: {name} analyses it")
            } else {
                "red: waits for you".into()
            }
        }
        "not published" => format!("not published: {}", b.why.as_deref().unwrap_or("")),
        "after failed" => format!("After failed: {}", b.why.as_deref().unwrap_or("")),
        bs::AFTER_WAITS => format!("After waits: {}", b.why.as_deref().unwrap_or("")),
        other => other.to_string(),
    })
}

/// What a card says of the queue, by task key: the batch's line on its integration task's card ("batch 7 ·
/// checking · 12 min"), "in batch 7", a place, "verifying" or "waits for Verify", "waits for API-12", "waits: the target is red (API-77)"; a task back in
/// work, "returned: …".
/// With the batch's number, so the page draws a batch's cards as one group; a task in work, the tasks it overlaps;
/// and the human's keys on the card (AC-193): "integrate" on a task in work of a project that integrates, "take-out" on
/// a queued one, a deciding batch's own.
pub fn marks(tx: &Transaction, tasks: &[Task], now: i64) -> Result<HashMap<String, Value>, String> {
    let mut out: HashMap<String, Value> = HashMap::new();
    // a batch's integration task keeps its head's line where it waits for its agent or the human
    let projects: HashSet<&str> = tasks.iter()
        .filter(|t| t.status == INTEGRATION || (is_integration(&t.id) && ["IN PROGRESS", "WAIT FOR HUMAN"].contains(&t.status.as_str())))
        .map(|t| t.project.as_str()).collect();
    for project in projects {
        for b in bs::of_project(tx, project, i64::MAX, 5)?.iter().filter(|b| !["ended", "given up"].contains(&b.state.as_str())) {
            let keys = keys_of(b);
            if let Some(task) = &b.task {
                let minutes = minutes_since(b.step_started.unwrap_or(b.created_at), now);
                out.insert(task.clone(), json!({"mark": format!("batch {} · {} · {minutes} min", b.number, state_words(tx, b)?), "batch": b.number, "head": true,
                                                "batch_id": b.id, "keys": keys}));
            }
            // each card of the group holds the batch's keys for the human: the same as its integration task's window
            for k in &b.tasks {
                out.insert(k.clone(), json!({"mark": format!("in batch {}", b.number), "batch": b.number, "batch_id": b.id, "keys": keys}));
            }
        }
        let waiting = ops::rows(tx, "SELECT e.task, e.stage, e.waits_for, EXISTS (SELECT 1 FROM integration_verifies v WHERE v.task = e.task)
                                     FROM integration_entries e JOIN tasks t ON t.id = e.task
                                     WHERE e.project = ?1 AND e.batch IS NULL ORDER BY e.queued_at, t.number", [project],
            |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, bool>(3)?)))?;
        let bug = waiting.iter().any(|w| w.1.as_deref() == Some(super::red_target::HELD)).then(|| super::red_target::open_bug(tx, project)).transpose()?.flatten();
        for (i, (task, stage, waits, verified)) in waiting.into_iter().enumerate() {
            let mark = match (waits.filter(|w| !w.is_empty()), stage) {
                (Some(w), _) => format!("waits for {w}"),
                (None, Some(s)) if s == super::red_target::HELD => bug.as_ref().map_or("waits: the target is red".into(), |b| format!("waits: the target is red ({b})")),
                (None, Some(_)) if verified => "verifying".to_string(),
                (None, Some(_)) => "waits for Verify".to_string(),
                (None, None) => ordinal(i + 1),
            };
            out.insert(task, json!({"mark": mark}));
        }
    }
    // a task back in work from the queue says how, until queued again or out of IN PROGRESS
    let working: Vec<&str> = tasks.iter().filter(|t| t.status == "IN PROGRESS").map(|t| t.key.as_str()).collect();
    let returned: Vec<String> = ops::rows(tx, "SELECT DISTINCT task FROM events WHERE task IN (SELECT value FROM json_each(?1)) AND kind = 'comment'
                                               AND json_extract(data, '$.left') IN ('red', 'conflict')", [serde_json::to_string(&working).unwrap_or_default()], |r| r.get(0))?;
    for key in returned {
        let words = match ops::how_left(tx, &key)?.as_deref() {
            Some("conflict") => "returned: conflict",
            Some("red") => "returned: red check",
            _ => continue,
        };
        out.insert(key, json!({"mark": words}));
    }
    // a task in work whose commits overlap others' says each (AC-193), as the last walk found it
    for key in working {
        let ids = super::notices::overlaps_of(key, now);
        if !ids.is_empty() {
            out.entry(key.to_string()).or_insert_with(|| json!({}))["overlaps"] = json!(ids);
        }
    }
    // Integrate and Take out, where terminumd would take them: never a batch's own task, which ends with its batch
    let on: HashSet<String> = ops::rows(tx, "SELECT project FROM project_integration WHERE is_on = 1", [], |r| r.get(0))?.into_iter().collect();
    for t in tasks.iter().filter(|t| !is_integration(&t.id)) {
        let key = match t.status.as_str() {
            "IN PROGRESS" if on.contains(&t.project) => "integrate",
            INTEGRATION => "take-out",
            _ => continue,
        };
        let mark = out.entry(t.key.clone()).or_insert_with(|| json!({}));
        let mut keys = vec![json!(key)];
        keys.extend(mark["keys"].as_array().cloned().unwrap_or_default());
        mark["keys"] = Value::Array(keys);
    }
    Ok(out)
}

/// Each project with integration on, its queue's word for its column's head: paused (by whom), or waiting (why).
pub fn notes(tx: &Transaction) -> Result<HashMap<String, Value>, String> {
    let rows = ops::rows(tx, "SELECT project, paused, waits_why FROM project_integration WHERE is_on = 1", [],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, bool>(1)?, r.get::<_, Option<String>>(2)?)))?;
    let human = ops::user_name(tx, HUMAN)?;
    rows.into_iter().map(|(p, paused, waits)| {
        let note = if paused { Some(format!("Paused by {human}")) } else if waits.is_some() { waits } else { after_note(tx, &p)? };
        Ok((p, json!({"paused": paused, "note": note})))
    }).collect()
}

/// A batch whose After waits, with the minutes since its first wait (AC-191).
fn after_note(tx: &Transaction, project: &str) -> Result<Option<String>, String> {
    let Some(b) = bs::after_waiting(tx, project)? else { return Ok(None) };
    let minutes = minutes_since(b.waits_since.unwrap_or(b.created_at), ops::now());
    Ok(Some(format!("Batch {}: After waits {minutes} min: {}", b.number, b.why.as_deref().unwrap_or(""))))
}

fn minutes_since(since: i64, now: i64) -> i64 {
    (now - since).max(0) / 60
}

/// Pause or Resume (AC-193): a paused queue starts no batch; the one in work goes on.
async fn pause(State(app): State<Shared>, Path(project): Path<String>, Json(body): Json<Value>) -> Response {
    let paused = body["paused"].as_bool().unwrap_or(true);
    let done = store(&app, move |tx| {
        ops::check_project(tx, &project)?;
        if tx.execute("UPDATE project_integration SET paused = ?2 WHERE project = ?1 AND is_on = 1", (&project, paused)).map_err(sql)? == 0 {
            return Err("Integration is off for this project.".into());
        }
        Ok(json!({"ok": true, "paused": paused}))
    }).await;
    told(&app, None, done)
}

/// The pages hear what a key changed; its answer.
fn told(app: &Shared, task: Option<&str>, done: Result<Value, String>) -> Response {
    if done.is_ok() {
        if let Some(id) = task {
            crate::tasks::api::told(app, id);
        }
        crate::page::projects::tell(app);
    }
    answer(done)
}

#[derive(serde::Deserialize)]
struct Before {
    before: Option<i64>,
}

async fn batches(State(app): State<Shared>, Path(project): Path<String>, Query(q): Query<Before>) -> Response {
    answer(store(&app, move |tx| batches_of(tx, &project, q.before)).await)
}

/// A project's batches, newest first, 100 at a time (AC-193), with how many of its last 20 were green.
fn batches_of(tx: &Transaction, project: &str, before: Option<i64>) -> Result<Value, String> {
    ops::check_project(tx, project)?;
    let page = bs::of_project(tx, project, before.unwrap_or(i64::MAX), BATCHES_PAGE)?;
    let mut out = Vec::new();
    for b in &page {
        let checked: Option<i64> = tx.query_row("SELECT sum(ended - started) FROM integration_rounds WHERE batch = ?1 AND ended IS NOT NULL AND started IS NOT NULL", [b.id], |r| r.get(0)).map_err(sql)?;
        let result = match b.state.as_str() {
            "ended" => {
                let green = if b.round > 1 { format!("green in round {}", b.round) } else { "green".into() };
                // what its After's turn said, or the human's words closing it (AC-191)
                b.why.as_deref().map_or(green.clone(), |w| format!("{green}; {w}"))
            }
            "given up" => format!("nothing published: {}", b.why.as_deref().unwrap_or_default()),
            other => other.to_string(),
        };
        let ended = ["ended", "given up"].contains(&b.state.as_str());
        out.push(json!({"number": b.number, "tasks": b.tasks.iter().filter_map(|k| bs::slug(tx, k).ok()).collect::<Vec<_>>(), "base": short(&b.base),
                        "candidate": b.candidate.as_deref().map(short), "task": bs::task_of(tx, b)?, "rounds": b.round, "result": result,
                        "ended": ended, "checked_seconds": checked, "log": format!("batch-{}.{}.check.log", b.number, b.round)}));
    }
    let last = bs::of_project(tx, project, i64::MAX, 20)?;
    let green = last.iter().filter(|b| b.state == "ended").count();
    Ok(json!({"batches": out, "green": green, "of": last.len(), "more": page.len() as i64 == BATCHES_PAGE}))
}

/// The folder a session's task works in, from its layout row: where its agent's hooks said it went, else the pane's;
/// none on another host, whose folders are not this Mac's.
fn folder_of(row: &Value) -> Option<String> {
    let (host, dir) = crate::page::marks::project_of(row);
    if host.is_some() {
        return None;
    }
    row["agentDir"].as_str().filter(|d| !d.is_empty()).map(str::to_string).or(dir)
}

/// A human's Integrate on an IN PROGRESS task (AC-193): its session's folder's HEAD; refused in integrate's words.
async fn queue_task(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let wanted = id.clone();
    let session = match store(&app, move |tx| Ok(ops::get(tx, &wanted)?.session)).await {
        Ok(s) => s,
        Err(e) => return answer::<Value>(Err(e)),
    };
    let folder = session.and_then(|key| folder_of(crate::page::marks::find_item(&app.layout.read(), &key)?));
    let (task, tasks) = (id.clone(), app.tasks.clone());
    let done = tokio::task::spawn_blocking(move || crate::tasks::integration_queue::integrate_by_human(&tasks, &task, folder.as_deref()))
        .await.map_err(|e| e.to_string()).and_then(|r| r);
    told(&app, Some(&id), done.map(|said| json!({"ok": true, "said": said})))
}

/// A human's Take out (AC-187, AC-193): the task IN PROGRESS again, out of the queue and any batch, which goes on
/// without it.
pub(crate) fn out(tx: &Transaction, id: &str, at: i64) -> Result<(), String> {
    let t = ops::get(tx, id)?;
    if t.status != INTEGRATION {
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
    told(&app, Some(&id), done.map(|()| json!({"ok": true})))
}

#[cfg(test)]
#[path = "column_tests.rs"]
mod tests;
