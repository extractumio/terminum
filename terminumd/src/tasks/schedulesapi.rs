// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Schedules' routes (`terminum-schedules.md` §9), behind the page's sign-in and write checks: the only way a
//! schedule is written. Every page hears `schedules` when one changes or fires.

use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use rusqlite::Transaction;
use serde::Deserialize;
use serde_json::{json, Value};

use super::api::{answer, store};
use super::ops::{self, rows};
use super::schedules::{self, Given};
use super::store::HUMAN;
use crate::app::Shared;

/// Firings a page of the Scheduled page reads (AC-164).
const FIRES_PAGE: i64 = 100;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/schedules", get(list).post(make))
        .route("/api/schedules/preview", post(preview))
        .route("/api/schedules/{id}", patch(change).delete(drop_one))
        .route("/api/schedule-fires", get(fires))
}

/// Every page hears a schedule changed or fired.
pub fn heard(app: &Shared) {
    app.pages.tell(&json!({"t": "schedules"}).to_string());
}

fn told(app: &Shared, done: Result<Value, String>) -> Response {
    if done.is_ok() {
        heard(app);
    }
    answer(done)
}

async fn list(State(app): State<Shared>) -> Response {
    answer(store(&app, |tx| Ok(json!({"schedules": schedules::listed(tx, ops::now())?}))).await)
}

async fn make(State(app): State<Shared>, Json(body): Json<Value>) -> Response {
    let done = store(&app, move |tx| {
        let now = ops::now();
        let (id, key) = schedules::add(tx, HUMAN, &Given::of(tx, &body, now)?, now)?;
        Ok(json!({"id": id, "key": key}))
    }).await;
    told(&app, done)
}

async fn change(State(app): State<Shared>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let done = store(&app, move |tx| {
        let now = ops::now();
        if schedules::change(tx, HUMAN, &id, &Given::of(tx, &body, now)?, now)? {
            crate::dispatcher::fire::called_off(tx, &id, "Its schedule was paused.")?;
        }
        Ok(json!({"ok": true}))
    }).await;
    crate::tasks::api::told(&app, "");
    told(&app, done)
}

async fn drop_one(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    let done = store(&app, move |tx| {
        schedules::remove(tx, &id, ops::now())?;
        crate::dispatcher::fire::called_off(tx, &id, "Its schedule was deleted.")?;
        Ok(json!({"ok": true}))
    }).await;
    crate::tasks::api::told(&app, "");
    told(&app, done)
}

/// What a schedule's window shows as it is typed (AC-160): its next three runs, the prompt and the agent's line,
/// or the field's words.
async fn preview(State(app): State<Shared>, Json(body): Json<Value>) -> Response {
    answer(store(&app, move |tx| {
        let now = ops::now();
        let given = Given::of(tx, &body, now)?;
        let s = given.as_schedule();
        let next = s.when()?.upcoming(now, 3);
        let line = super::agents::by_id(tx, &s.agent)?.map(|a| a.line).unwrap_or_default();
        let next_words: Vec<String> = next.iter().map(|at| super::cron::when_words(*at)).collect();
        Ok(json!({"next": next, "next_words": next_words, "words": s.words(), "prompt": crate::dispatcher::fire::prompt(&s, "<its new task>", &s.title, &s.body, false), "line": line}))
    }).await)
}

#[derive(Deserialize)]
struct Asked {
    schedule: Option<String>,
    before: Option<i64>,
}

/// The Scheduled page's rows, newest first (AC-164).
async fn fires(State(app): State<Shared>, Query(asked): Query<Asked>) -> Response {
    answer(store(&app, move |tx| {
        let mut found = fires_of(tx, asked.schedule.as_deref(), asked.before, FIRES_PAGE + 1)?;
        let more = found.len() as i64 > FIRES_PAGE;
        found.truncate(FIRES_PAGE as usize);
        Ok(json!({"fires": found, "more": more}))
    }).await)
}

/// Firings of `schedule` ("SCH-3"; none: all), those older than `before`.
pub(crate) fn fires_of(tx: &Transaction, schedule: Option<&str>, before: Option<i64>, most: i64) -> Result<Vec<Value>, String> {
    let number = schedule.map(|k| k.trim_start_matches("SCH-").parse::<i64>().map_err(|_| format!("No schedule {k}"))).transpose()?;
    // the firing keeps what went in; the task's first run says on which agent, its moves when it worked, its last
    // word by an agent or Dispatcher what came out
    rows(tx, "SELECT f.id, 'SCH-' || s.number, s.title, f.due_at, f.at, t.slug, t.status, f.state, f.missed, f.why,
                     (SELECT r.agent_name FROM run_tasks rt JOIN runs r ON r.id = rt.run WHERE rt.task = t.id ORDER BY r.rowid LIMIT 1),
                     f.prompt,
                     (SELECT min(e.at) FROM events e WHERE e.task = t.id AND e.kind = 'move' AND json_extract(e.data, '$.to') = 'IN PROGRESS'),
                     (SELECT max(e.at) FROM events e WHERE e.task = t.id AND e.kind = 'move' AND json_extract(e.data, '$.to') IN ('DONE', 'FAILED')),
                     (SELECT e.text FROM events e JOIN users u ON u.id = e.author WHERE e.task = t.id AND e.kind IN ('comment', 'told') AND u.kind <> 'human'
                        ORDER BY e.at DESC, e.rowid DESC LIMIT 1)
              FROM schedule_fires f JOIN schedules s ON s.id = f.schedule LEFT JOIN tasks t ON t.id = f.task
              WHERE (?1 IS NULL OR s.number = ?1) AND (?2 IS NULL OR f.id < ?2) ORDER BY f.id DESC LIMIT ?3",
         rusqlite::params![number, before, most], |r| Ok(json!({
             "id": r.get::<_, i64>(0)?, "schedule": r.get::<_, String>(1)?, "title": r.get::<_, String>(2)?, "due_at": r.get::<_, i64>(3)?, "at": r.get::<_, i64>(4)?,
             "task": r.get::<_, Option<String>>(5)?, "status": r.get::<_, Option<String>>(6)?, "state": r.get::<_, String>(7)?, "missed": r.get::<_, i64>(8)?,
             "why": r.get::<_, Option<String>>(9)?, "agent": r.get::<_, Option<String>>(10)?, "prompt": r.get::<_, Option<String>>(11)?,
             "started_at": r.get::<_, Option<i64>>(12)?, "ended_at": r.get::<_, Option<i64>>(13)?, "result": r.get::<_, Option<String>>(14)?,
         })))
}
