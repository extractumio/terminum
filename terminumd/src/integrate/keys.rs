// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A human's keys on a batch (`terminum-integration.md` AC-194, AC-191): for one that is red or not published, or
//! whose After failed, so no batch holds its project's queue for good. Each in one transaction; what needs git
//! (a batch built anew without a task) the engine does at its next step.

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use rusqlite::Transaction;
use serde_json::{json, Value};

use super::store::{self as bs, Batch};
use crate::app::Shared;
use crate::tasks::api::{answer, store};
use crate::tasks::ops;
use crate::tasks::store::{DISPATCHER, HUMAN};

/// The states a human decides in.
const DECIDE: [&str; 3] = ["red", "not published", "after failed"];

fn deciding(tx: &Transaction, id: i64) -> Result<Batch, String> {
    let b = bs::by_id(tx, id)?;
    if DECIDE.contains(&b.state.as_str()) { Ok(b) } else { Err(format!("Batch {} is {}: nothing to decide now.", b.number, b.state)) }
}

fn close_task(tx: &Transaction, b: &Batch, words: &str, at: i64) -> Result<(), String> {
    let id = bs::slug(tx, b.task.as_deref().unwrap_or_default())?;
    ops::comment(tx, HUMAN, &id, words, at)?;
    let t = ops::get(tx, &id)?;
    ops::place(tx, DISPATCHER, &t, "DONE", at)
}

/// Every task of a red or unpublished batch back to its agent; its integration task DONE.
pub fn return_all(tx: &Transaction, id: i64, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    if b.state == "after failed" {
        return Err(format!("Batch {} is published: its tasks are DONE. Close it.", b.number));
    }
    let who = ops::user_name(tx, HUMAN)?;
    for k in &b.tasks {
        bs::give_back(tx, k, &format!("{} returned: batch {} was given up by {who}. Integrate again when you are told to, or ask.", bs::slug(tx, k)?, b.number), bs::Left::Out, at)?;
    }
    bs::set_state(tx, b.id, "given up", Some(&format!("given up by {who}")), at)?;
    close_task(tx, &b, &format!("Batch {}: nothing published, its {} tasks returned by {who}.", b.number, b.tasks.len()), at)?;
    Ok(format!("Batch {}'s tasks went back to their agents.", b.number))
}

/// One task of the batch back to its agent with the human's words; the rest built anew and checked.
pub fn return_one(tx: &Transaction, id: i64, task: &str, why: &str, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    let t = ops::get(tx, task)?;
    if !b.tasks.contains(&t.key) {
        return Err(format!("{task} is not in batch {}.", b.number));
    }
    if why.trim().is_empty() {
        return Err("Say why, for its agent.".into());
    }
    let who = ops::user_name(tx, HUMAN)?;
    // leaving the queue takes it out of the batch, and gives the batch up with its last task (store::left)
    // from a red batch, the human returns it for its check, as its agent would; from one not published, it is taken out
    let how = if b.state == bs::RED { bs::Left::Red } else { bs::Left::Out };
    bs::give_back(tx, &t.key, &format!("{} returned by {who} from batch {}: {}", t.id, b.number, why.trim()), how, at)?;
    let b = bs::by_id(tx, b.id)?;
    if b.state == bs::RED {
        bs::check_again(tx, &b, None, at)?;
    } else if b.state == "not published" {
        bs::rebuild(tx, &b, at)?;
    }
    Ok(format!("{} went back to its agent.", t.id))
}

/// The same candidate checked again, a new round; never refused for a number of rounds.
pub fn check_again(tx: &Transaction, id: i64, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    if b.state != "red" {
        return Err(format!("Batch {} is {}: Check again is for a red batch.", b.number, b.state));
    }
    bs::check_again(tx, &b, None, at)
}

/// A batch not published tried again: the push or the project's Publish once more (no new check: it was green).
pub fn publish_again(tx: &Transaction, id: i64, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    if b.state != "not published" {
        return Err(format!("Batch {} is {}: Publish again is for one not published.", b.number, b.state));
    }
    bs::set_state(tx, b.id, "green", None, at)?;
    bs::back_to_work(tx, &b, at)?;
    Ok(format!("Batch {} is published again.", b.number))
}

/// A published batch whose After failed, closed with the human's words.
pub fn close(tx: &Transaction, id: i64, why: &str, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    if b.state != "after failed" {
        return Err(format!("Batch {} is {}: decide its tasks first.", b.number, b.state));
    }
    bs::set_state(tx, b.id, "ended", Some(why), at)?;
    close_task(tx, &b, if why.trim().is_empty() { "Closed." } else { why.trim() }, at)?;
    Ok(format!("Batch {} is closed.", b.number))
}

/// A batch whose After failed: After run once more in the tail's turn, before its project's later Afters, its folder
/// moved only if it does not hold the batch's commit yet; its integration task back in INTEGRATION until After passes or fails again (AC-191). A human's key only.
pub fn after_again(tx: &Transaction, id: i64, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    if b.state != bs::AFTER_FAILED {
        return Err(format!("Batch {} is {}: After again is for a batch whose After failed.", b.number, b.state));
    }
    let who = ops::user_name(tx, HUMAN)?;
    bs::after_again(tx, &b, &format!("After runs again, by {who}"))?;
    if let Some(task) = bs::task_of(tx, &b)? {
        ops::comment(tx, HUMAN, &task, &format!("After runs again for batch {}, by {who}.", b.number), at)?;
    }
    bs::back_to_work(tx, &b, at)?;
    Ok(format!("Batch {}'s After runs again.", b.number))
}

/// The keys a human has on batch `b` in its state, as its integration task's window and its group's cards show them (AC-193).
pub fn keys_of(b: &Batch) -> &'static [&'static str] {
    match b.state.as_str() {
        "red" => &["check-again", "return", "return-all"],
        "not published" => &["publish-again", "return", "return-all"],
        "after failed" => &["after-again", "close"],
        _ => &[],
    }
}

/// A batch as its integration task's window shows it.
fn of_task(tx: &Transaction, task: &str) -> Result<Value, String> {
    let b = bs::by_task(tx, &ops::key_of(tx, task)?)?.ok_or_else(|| format!("{task} is no batch's task."))?;
    let tasks: Vec<Value> = b.tasks.iter().map(|k| { let t = ops::get(tx, &bs::slug(tx, k)?)?; Ok(json!({"id": t.id, "status": t.status})) }).collect::<Result<_, String>>()?;
    Ok(json!({"batch": b.id, "number": b.number, "state": b.state, "why": b.why, "round": b.round, "red_rounds": b.red_rounds, "tasks": tasks, "keys": keys_of(&b), "pane": b.pane}))
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/integration/task/{id}", get(shown))
        .route("/api/integration/batch/{id}/{key}", post(pressed))
}

async fn shown(State(app): State<Shared>, Path(task): Path<String>) -> Response {
    let mut shown = store(&app, move |tx| of_task(tx, &task)).await;
    // its step's window is a session of terminumd's own tmux: its key, so the window can show it (AC-193)
    if let Ok(v) = &mut shown && let Some(pane) = v["pane"].as_str().map(str::to_string) {
        v["window"] = json!(app.tmux.local.key(super::runner::pane_of(&pane)));
    }
    answer(shown)
}

async fn pressed(State(app): State<Shared>, Path((id, key)): Path<(i64, String)>, body: Option<Json<Value>>) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or_default();
    let at = ops::now();
    let done = store(&app, move |tx| match key.as_str() {
        "return-all" => return_all(tx, id, at),
        "return" => return_one(tx, id, body["task"].as_str().unwrap_or_default(), body["why"].as_str().unwrap_or_default(), at),
        "check-again" => check_again(tx, id, at),
        "publish-again" => publish_again(tx, id, at),
        "after-again" => after_again(tx, id, at),
        "close" => close(tx, id, body["why"].as_str().unwrap_or_default(), at),
        other => Err(format!("No key {other}.")),
    }).await;
    if done.is_ok() {
        crate::page::projects::tell(&app);
    }
    answer(done.map(|said| json!({"ok": true, "said": said})))
}
