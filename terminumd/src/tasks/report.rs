// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A report waits for its human (terminum-dispatcher.md AC-264): a result for a person to read stands in WAIT FOR
//! HUMAN, its move marked `report`; for its run it is finished work, no agent moves it, and the human's Done moves it
//! to DONE.

use axum::extract::{Path, State};
use axum::response::Response;
use axum::routing::post;
use axum::Router;
use rusqlite::Transaction;
use serde_json::{json, Value};

use super::ops::{self, rows, sql, Task};
use super::store::{DISPATCHER, HUMAN};
use crate::app::Shared;

/// The field a report's move keeps.
const MARK: &str = "report";

/// The SQL of task alias `$t` waiting with its report: in WAIT FOR HUMAN, its newest move the report's.
macro_rules! waits {
    ($t:literal) => { concat!("(", $t, ".status = 'WAIT FOR HUMAN' AND coalesce((SELECT json_extract(rw.data, '$.report') FROM events rw WHERE rw.task = ",
                              $t, ".id AND rw.kind = 'move' ORDER BY rw.rowid DESC LIMIT 1), 0) = 1)") };
}
pub(crate) use waits;

/// Whether task `key` waits with its report.
pub fn waits_on(tx: &Transaction, key: &str) -> Result<bool, String> {
    tx.query_row(concat!("SELECT ", waits!("t"), " FROM tasks t WHERE t.id = ?1"), [key], |r| r.get(0)).map_err(sql)
}

/// Whether finish_task's result for task `key` is a report: the agent says so, and a schedule's task's always is.
pub fn wanted(tx: &Transaction, key: &str, args: &Value) -> Result<bool, String> {
    Ok(args["report"] == "yes" || args["report"] == true || super::schedules::of_task(tx, key)?.is_some())
}

/// Refused to every agent: a waiting report is moved by its human's Done or RUN alone.
pub fn untouched(tx: &Transaction, t: &Task) -> Result<(), String> {
    if waits_on(tx, &t.key)? {
        return Err(format!("{} waits for the human to read its report: only their Done or RUN moves it. You can comment.", t.id));
    }
    Ok(())
}

/// `t` waits with its report, moved by `author`; an older report of its schedule still unread is DONE, superseded:
/// one report of a schedule waits at a time. The superseded tasks' IDs.
pub fn wait(tx: &Transaction, author: &str, t: &Task, at: i64) -> Result<Vec<String>, String> {
    ops::move_marked(tx, author, &t.id, "WAIT FOR HUMAN", Some((MARK, "a report to read")), at)?;
    let Some(s) = super::schedules::of_task(tx, &t.key)? else { return Ok(Vec::new()) };
    let older: Vec<String> = rows(tx, concat!("SELECT t.slug FROM tasks t JOIN schedule_fires f ON f.task = t.id
                                               WHERE f.schedule = ?1 AND t.id <> ?2 AND t.archived_at IS NULL AND ", waits!("t"), " ORDER BY t.number"),
                                  (&s.id, &t.key), |r| r.get(0))?;
    for id in &older {
        ops::comment(tx, DISPATCHER, id, &format!("Superseded by {}'s report.", t.id), at)?;
        ops::move_to(tx, DISPATCHER, id, "DONE", at)?;
    }
    Ok(older)
}

/// The human read task `id`'s report: DONE, the one move a human makes but by running (terminum-kanban.md AC-107).
pub fn read(tx: &Transaction, id: &str, at: i64) -> Result<(), String> {
    let t = ops::get(tx, id)?;
    if !waits_on(tx, &t.key)? {
        return Err(if t.status == "WAIT FOR HUMAN" {
            format!("{id} has questions for you, not a report: answer them under Comment and press RUN.")
        } else {
            format!("{id} has no report waiting: a task moves by running it.")
        });
    }
    ops::move_marked(tx, HUMAN, id, "DONE", Some(("read", "the report was read")), at)
}

pub fn routes() -> Router<Shared> {
    Router::new().route("/api/tasks/{id}/read", post(read_route))
}

/// The window's Done on a waiting report.
async fn read_route(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let wanted = id.clone();
    let done = super::api::store(&app, move |tx| read(tx, &wanted, ops::now())).await;
    if done.is_ok() {
        super::api::told(&app, &id);
    }
    super::api::answer(done.map(|()| json!({"ok": true})))
}

#[cfg(test)]
#[path = "report_tests.rs"]
mod tests;
