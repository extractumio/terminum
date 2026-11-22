// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-223: signed-in reads of one project's scoreboard and retained signal evidence.

use axum::{Router, extract::{Path, Query, State}, response::Response, routing::get};
use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::app::Shared;
use crate::http::off;
use crate::tasks::{api::answer, ops::{check_project, now, project_place}};

use super::{SCHEMA, Signal, flow, scoreboard, settings, signals};

const PAGE: usize = 50;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/task-projects/{id}/supervisor/screen", get(screen))
        .route("/api/task-projects/{id}/supervisor/signals", get(more))
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
struct SignalPage {
    signals: Vec<Signal>,
    next: Option<String>,
}

#[derive(Serialize)]
struct Screen {
    project_name: String,
    indicators: Vec<Indicator>,
    #[serde(flatten)]
    signals: SignalPage,
}

fn signal_page(tx: &Transaction, id: &str, after: Option<&str>) -> Result<SignalPage, String> {
    let mut signals = signals::page(tx, id, after, PAGE + 1)?;
    let more = signals.len() > PAGE;
    signals.truncate(PAGE);
    let next = signals.last().filter(|_| more).map(|s| s.fingerprint.clone());
    Ok(SignalPage { signals, next })
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

async fn screen(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        check_project(tx, &id)?;
        let (project_name, _, _) = project_place(tx, &id)?;
        Ok(Screen { project_name, indicators: indicators(tx, &id)?, signals: signal_page(tx, &id, None)? })
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
