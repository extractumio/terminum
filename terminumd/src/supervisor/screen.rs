// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-223: signed-in reads of one project's scoreboard and retained signal evidence.

use axum::{Router, extract::{Path, Query, State}, response::Response, routing::{get, post}, Json};
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::app::Shared;
use crate::http::off;
use crate::tasks::{api::{answer, store}, ops::{check_project, now, project_place}};

use super::{SCHEMA, Signal, flow, governor, register, scoreboard, rounds, settings, signals};

const PAGE: usize = 50;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-projects/{id}/supervisor/screen", get(screen))
        .route("/api/task-projects/{id}/supervisor/signals", get(more))
        .route("/api/task-projects/{id}/supervisor/problems/{problem}/ruling", post(ruling))
        .route("/api/task-projects/{id}/supervisor/breakers/{breaker}/review", post(review_breaker))
        .route("/api/task-projects/{id}/supervisor/breakers/{breaker}/restore", post(restore_breaker))
}

#[derive(Serialize)]
struct Metric {
    name: &'static str,
    verdict: Option<&'static str>,
    before: Option<f64>,
    current: Option<f64>,
    noise: f64,
    missing: Option<&'static str>,
}

#[derive(Serialize)]
struct Indicator {
    name: &'static str,
    verdict: &'static str,
    guard: &'static str,
    metrics: Vec<Metric>,
}

#[derive(Serialize)]
struct ShownSignal {
    #[serde(flatten)]
    signal: Signal,
    investigation: Option<String>,
}

#[derive(Serialize)]
struct SignalPage {
    signals: Vec<ShownSignal>,
    investigations_enabled: bool,
    next: Option<String>,
}

#[derive(Serialize)]
struct Screen {
    project_name: String,
    holds: Vec<super::holds::Hold>,
    breakers: Vec<super::breakers::Open>,
    outcomes: Vec<super::outcomes::Score>,
    indicators: Vec<Indicator>,
    problems: Vec<register::Problem>,
    spending: governor::Spending,
    marked_tasks_eligible: bool,
    decisions: Vec<super::decisions::Decision>,
    digest: Option<super::digest::Digest>,
    review: Option<super::review::Published>,
    review_setup_error: Option<String>,
    #[serde(flatten)]
    signals: SignalPage,
}

fn signal_page(tx: &Transaction, id: &str, after: Option<&str>) -> Result<SignalPage, String> {
    let mut signals = signals::page(tx, id, after, PAGE + 1)?;
    let more = signals.len() > PAGE;
    signals.truncate(PAGE);
    let next = signals.last().filter(|_| more).map(|s| s.fingerprint.clone());
    let signals = signals.into_iter().map(|signal| {
        Ok::<_, String>(ShownSignal { investigation: rounds::status(tx, &signal.fingerprint)?, signal })
    }).collect::<Result<_, _>>()?;
    Ok(SignalPage { signals, next, investigations_enabled: rounds::enabled(tx, id)?.is_some() })
}

fn indicators(tx: &Transaction, id: &str) -> Result<Vec<Indicator>, String> {
    let active = settings::read(tx, id)?.settings.enabled_indicators();
    if !active.contains(&true) {
        return Ok(Vec::new());
    }
    let at = now();
    let (first, _) = flow::history_bounds(tx, id, at)?;
    let weeks = scoreboard::read(tx, id, at, first)?;
    Ok(scoreboard::indicators(&weeks).into_iter().zip(active)
        .filter(|(_, enabled)| *enabled)
        .map(|(i, _)| Indicator {
            name: i.name, verdict: i.verdict.label(), guard: i.guard,
            metrics: i.lines.into_iter().map(|m| Metric {
                name: m.name,
                verdict: (!m.context).then(|| m.verdict.label()),
                before: m.before, current: m.current, noise: m.noise,
                missing: m.before.zip(m.current).is_none().then_some(m.missing),
            }).collect(),
        }).collect())
}

#[derive(Deserialize)]
struct ScreenQuery { signal: Option<String> }

async fn screen(State(app): State<Shared>, Path(id): Path<String>, Query(query): Query<ScreenQuery>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        check_project(tx, &id)?;
        let (project_name, _, _) = project_place(tx, &id)?;
        let at = now();
        let mut page = signal_page(tx, &id, None)?;
        if let Some(key) = query.signal.filter(|key| page.signals.iter().all(|entry| entry.signal.fingerprint != *key)) {
            let signal = signals::get(tx, &id, &key)?;
            page.signals.insert(0, ShownSignal { signal, investigation: rounds::status(tx, &key)? });
        }
        let review_setup_error = tx.query_row("SELECT (SELECT review_setup_error FROM project_supervisor WHERE project = ?1)",
            [&id], |r| r.get(0)).map_err(crate::tasks::ops::sql)?;
        Ok(Screen { project_name, holds: super::holds::list(tx, &id)?, indicators: indicators(tx, &id)?, signals: page,
            breakers: super::breakers::list(tx, &id)?, outcomes: super::outcomes::scores(tx, &id)?,
            problems: register::ranked(tx, &id, at, true)?, spending: governor::spending(tx, &id, at)?,
            marked_tasks_eligible: crate::tasks::auto::setting(tx, &id)?.supervisor_tasks,
            decisions: super::decisions::cards(tx, &id)?, digest: super::digest::latest(tx, &id)?,
            review: super::review::latest(tx, &id)?, review_setup_error })

    })).await)
}

#[derive(Deserialize)]
struct Cursor {
    after: String,
}

async fn more(State(app): State<Shared>, Path(id): Path<String>, Query(cursor): Query<Cursor>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        check_project(tx, &id)?;
        signal_page(tx, &id, Some(&cursor.after))
    })).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ruling { reason: String }

async fn ruling(State(app): State<Shared>, Path((project, problem)): Path<(String, i64)>, Json(input): Json<Ruling>) -> Response {
    let result = store(&app, move |tx| {
        register::rule(tx, &project, problem, &input.reason, now())?;
        Ok(serde_json::json!({"ok": true}))
    }).await;
    if result.is_ok() { app.supervisor.wake(); }
    answer(result)
}

async fn review_breaker(State(app): State<Shared>, Path((project, id)): Path<(String, i64)>) -> Response {
    let (reader, scope) = (app.clone(), project.clone());
    let review = off(move || reader.tasks.snapshot(SCHEMA,
        |tx| super::breakers::review(tx, &scope, id, now()))).await;
    answer(match review {
        Ok(review) => store(&app, move |tx| {
            let proof = super::breakers::save_review(tx, &project, id, &review)?;
            Ok(serde_json::json!({"review": review, "proof": proof}))
        }).await,
        Err(why) => Err(why),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Restore { proof: String }

async fn restore_breaker(State(app): State<Shared>, Path((project, id)): Path<(String, i64)>,
    Json(input): Json<Restore>) -> Response
{
    let result = store(&app, move |tx| {
        super::breakers::restore(tx, &project, id, &input.proof, now())?;
        Ok(serde_json::json!({"ok": true}))
    }).await;
    if result.is_ok() { app.supervisor.wake(); }
    answer(result)
}
