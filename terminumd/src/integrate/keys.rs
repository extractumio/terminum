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
use crate::tasks::ops::{self, sql, INTEGRATION};
use crate::tasks::store::{DISPATCHER, HUMAN};

/// The states a human decides in.
const DECIDE: [&str; 3] = ["red", "not published", "after failed"];

fn deciding(tx: &Transaction, id: i64) -> Result<Batch, String> {
    let b = bs::by_id(tx, id)?;
    if DECIDE.contains(&b.state.as_str()) { Ok(b) } else { Err(format!("Batch {} is {}: nothing to decide now.", b.number, b.state)) }
}

/// Its integration task back in INTEGRATION while terminumd works on its batch again.
fn back_to_work(tx: &Transaction, b: &Batch, at: i64) -> Result<(), String> {
    let t = ops::get(tx, &bs::slug(tx, b.task.as_deref().unwrap_or_default())?)?;
    ops::place(tx, DISPATCHER, &t, INTEGRATION, at)
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
        bs::give_back(tx, k, &format!("{} returned: batch {} was given up by {who}. Integrate again when you are told to, or ask.", bs::slug(tx, k)?, b.number), at)?;
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
    bs::give_back(tx, &t.key, &format!("{} returned by {who} from batch {}: {}", t.id, b.number, why.trim()), at)?;
    let rest: Vec<&String> = b.tasks.iter().filter(|k| **k != t.key).collect();
    if rest.is_empty() {
        bs::set_state(tx, b.id, "given up", Some("its last task returned"), at)?;
        close_task(tx, &b, &format!("Batch {}: nothing published, its tasks returned.", b.number), at)?;
    } else {
        tx.execute("UPDATE integration_batches SET tasks = ?2, state = 'rebuild', pane = NULL WHERE id = ?1", (b.id, serde_json::to_string(&rest).unwrap_or_default())).map_err(sql)?;
        back_to_work(tx, &b, at)?;
    }
    Ok(format!("{} went back to its agent.", t.id))
}

/// The same candidate checked again, a new round; never refused for a number of rounds.
pub fn check_again(tx: &Transaction, id: i64, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    if b.state != "red" {
        return Err(format!("Batch {} is {}: Check again is for a red batch.", b.number, b.state));
    }
    tx.execute("UPDATE integration_batches SET state = 'checking', pane = NULL, round = round + 1 WHERE id = ?1", [b.id]).map_err(sql)?;
    tx.execute("INSERT INTO integration_rounds (batch, round, tasks, base_sha, candidate_sha, result) VALUES (?1, ?2, ?3, ?4, ?5, 'built')",
               rusqlite::params![b.id, b.round + 1, serde_json::to_string(&b.tasks).unwrap_or_default(), b.base, b.candidate]).map_err(sql)?;
    back_to_work(tx, &b, at)?;
    Ok(format!("Batch {} is checked again: round {}.", b.number, b.round + 1))
}

/// A batch not published tried again: the push or the project's Publish once more (no new check: it was green).
pub fn publish_again(tx: &Transaction, id: i64, at: i64) -> Result<String, String> {
    let b = deciding(tx, id)?;
    if b.state != "not published" {
        return Err(format!("Batch {} is {}: Publish again is for one not published.", b.number, b.state));
    }
    bs::set_state(tx, b.id, "green", None, at)?;
    back_to_work(tx, &b, at)?;
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

/// A batch as its integration task's window shows it.
fn of_task(tx: &Transaction, task: &str) -> Result<Value, String> {
    let key = ops::key_of(tx, task)?;
    let id: i64 = tx.query_row("SELECT id FROM integration_batches WHERE task = ?1", [&key], |r| r.get(0)).map_err(|_| format!("{task} is no batch's task."))?;
    let b = bs::by_id(tx, id)?;
    let why: Option<String> = tx.query_row("SELECT why FROM integration_batches WHERE id = ?1", [id], |r| r.get(0)).map_err(sql)?;
    let tasks: Vec<Value> = b.tasks.iter().map(|k| { let t = ops::get(tx, &bs::slug(tx, k)?)?; Ok(json!({"id": t.id, "status": t.status})) }).collect::<Result<_, String>>()?;
    let keys: Vec<&str> = match b.state.as_str() {
        "red" => vec!["check-again", "return", "return-all"],
        "not published" => vec!["publish-again", "return", "return-all"],
        "after failed" => vec!["close"],
        _ => vec![],
    };
    Ok(json!({"batch": b.id, "number": b.number, "state": b.state, "why": why, "round": b.round, "red_rounds": b.red_rounds, "tasks": tasks, "keys": keys}))
}

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/integration/task/{id}", get(shown))
        .route("/api/integration/batch/{id}/{key}", post(pressed))
}

async fn shown(State(app): State<Shared>, Path(task): Path<String>) -> Response {
    answer(store(&app, move |tx| of_task(tx, &task)).await)
}

async fn pressed(State(app): State<Shared>, Path((id, key)): Path<(i64, String)>, body: Option<Json<Value>>) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or_default();
    let at = ops::now();
    let done = store(&app, move |tx| match key.as_str() {
        "return-all" => return_all(tx, id, at),
        "return" => return_one(tx, id, body["task"].as_str().unwrap_or_default(), body["why"].as_str().unwrap_or_default(), at),
        "check-again" => check_again(tx, id, at),
        "publish-again" => publish_again(tx, id, at),
        "close" => close(tx, id, body["why"].as_str().unwrap_or_default(), at),
        other => Err(format!("No key {other}.")),
    }).await;
    if done.is_ok() {
        crate::page::projects::tell(&app);
    }
    answer(done.map(|said| json!({"ok": true, "said": said})))
}
