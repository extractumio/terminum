// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-224–226: incident reads and owner actions on the signed-in page listener.

use axum::{Json, Router, extract::{Path, Query, State}, response::Response, routing::get};
use rusqlite::{OptionalExtension, Row, Transaction};
use std::time::Duration;
use serde::{Deserialize, Serialize};

use crate::app::Shared;
use crate::http::off;
use crate::tasks::{api::{answer, store}, ops::{check_project, now, rows, sql}};

use super::{SCHEMA, incidents::{self, Action}};

const PAGE: usize = 50;

pub(super) fn routes() -> Router<Shared> {
    Router::new()
        .route("/api/supervisor/incidents", get(attention))
        .route("/api/supervisor/incidents/watch", get(watch))
        .route("/api/supervisor/incidents/{id}", get(timeline).post(act))
        .route("/api/task-projects/{project}/supervisor/incidents", get(history))
}

#[derive(Debug, Serialize)]
pub(super) struct Incident {
    pub id: i64,
    pub project: String,
    project_name: String,
    level: u8,
    opened_at: i64,
    pub closed_at: Option<i64>,
    pub acknowledged_at: Option<i64>,
    pub snoozed_until: Option<i64>,
    revision: i64,
    pub happened: String,
    held: &'static str,
    needed: &'static str,
    no_answer: &'static str,
}

const SUMMARY: &str = "SELECT i.id, i.project, p.name, m.level,
      i.opened_at, i.closed_at, i.acknowledged_at, i.snoozed_until,
      (SELECT max(id) FROM supervisor_incident_events WHERE incident = i.id),
      m.metric, m.value, m.threshold,
      (SELECT count(*) FROM supervisor_incident_events WHERE incident = i.id AND kind = 'signal'),
      (SELECT related_id FROM supervisor_incident_events WHERE incident = i.id AND kind = 'continued')
    FROM supervisor_incidents i JOIN projects p ON p.id = i.project
    JOIN supervisor_incident_events m ON m.id = coalesce(
      (SELECT event FROM supervisor_incident_signals WHERE incident = i.id ORDER BY level DESC, event DESC LIMIT 1),
      (SELECT id FROM supervisor_incident_events WHERE incident = i.id AND value IS NOT NULL ORDER BY id DESC LIMIT 1))";

fn measurement(metric: &str, value: f64, threshold: f64) -> String {
    match metric {
        "missed_beats" => format!("Supervisor collector missed {value:.0} heartbeats."),
        "failed_deliveries" => format!("Critical line failed {value:.0} deliveries."),
        _ => format!("{} measured {value:.3} against a threshold of {threshold:.3}.", metric.replace('_', " ")),
    }
}

fn incident(row: &Row<'_>) -> rusqlite::Result<Incident> {
    let closed_at: Option<i64> = row.get(5)?;
    let acknowledged_at: Option<i64> = row.get(6)?;
    let continued: Option<i64> = row.get(13)?;
    let measured = measurement(&row.get::<_, String>(9)?, row.get(10)?, row.get(11)?);
    let count: i64 = row.get(12)?;
    let status = match (closed_at, continued) {
        (_, Some(id)) => format!("Continued in incident {id}. "),
        (Some(_), None) => "All-clear. ".into(),
        (None, None) => String::new(),
    };
    Ok(Incident {
        id: row.get(0)?, project: row.get(1)?, project_name: row.get(2)?, level: row.get(3)?,
        opened_at: row.get(4)?, closed_at, acknowledged_at, snoozed_until: row.get(7)?, revision: row.get(8)?,
        happened: format!("{status}{measured} {count} {} recorded.", if count == 1 { "signal" } else { "signals" }),
        held: "No hold in effect.",
        needed: if acknowledged_at.is_some() { "Acknowledged." } else { "Acknowledge when seen." },
        no_answer: "No automatic action without an answer.",
    })
}

pub(super) fn pending(tx: &Transaction) -> Result<Vec<Incident>, String> {
    rows(tx, &format!("{SUMMARY} WHERE i.closed_at IS NULL OR i.acknowledged_at IS NULL ORDER BY i.id DESC"), [], incident)
}

async fn attention(State(app): State<Shared>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, pending)).await)
}

#[derive(Deserialize)]
struct Watch { after: Option<u64> }

// A wake-up carries no incident data. The next feed read goes through the sign-in guard again.
async fn watch(State(app): State<Shared>, Query(watch): Query<Watch>) -> Json<u64> {
    let mut updates = app.supervisor.incident_updates.subscribe();
    if watch.after == Some(*updates.borrow_and_update()) {
        let _ = tokio::time::timeout(Duration::from_secs(20), updates.changed()).await;
    }
    let current = *updates.borrow_and_update();
    Json(current)
}

#[derive(Default, Deserialize)]
struct Cursor { before: Option<i64>, selected: Option<i64> }

#[derive(Serialize)]
struct Page<T> { rows: Vec<T>, next: Option<i64> }

fn page<T>(mut rows: Vec<T>, id: impl FnOnce(&T) -> i64) -> Page<T> {
    let more = rows.len() > PAGE;
    rows.truncate(PAGE);
    let next = rows.last().filter(|_| more).map(id);
    Page { rows, next }
}

async fn history(State(app): State<Shared>, Path(project): Path<String>, Query(cursor): Query<Cursor>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        check_project(tx, &project)?;
        let listed = rows(tx, &format!("{SUMMARY} WHERE i.project = ?1 AND i.id < ?2 ORDER BY i.id DESC LIMIT ?3"),
            (&project, cursor.before.unwrap_or(i64::MAX), PAGE as i64 + 1), incident)?;
        let mut page = page(listed, |row| row.id);
        if let Some(id) = cursor.selected.filter(|id| page.rows.iter().all(|row| row.id != *id)) {
            page.rows.extend(rows(tx, &format!("{SUMMARY} WHERE i.id = ?1 AND i.project = ?2"), (id, &project), incident)?);
        }
        Ok(page)
    })).await)
}

#[derive(Debug, Serialize)]
pub(super) struct Event {
    id: i64,
    pub at: i64,
    pub kind: String,
    pub actor: String,
    pub level: Option<u8>,
    pub words: String,
}

pub(super) fn events(tx: &Transaction, id: i64, before: Option<i64>) -> Result<Vec<Event>, String> {
    rows(tx, "SELECT id, at, kind, actor, level, metric, value, threshold, related_id
        FROM supervisor_incident_events WHERE incident = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
        (id, before.unwrap_or(i64::MAX), PAGE as i64 + 1), |r| {
            let kind: String = r.get(2)?;
            let level: Option<u8> = r.get(4)?;
            let words = match kind.as_str() {
                "signal" | "updated" => format!("Level {}. {}", level.unwrap_or(0),
                    measurement(&r.get::<_, String>(5)?, r.get(6)?, r.get(7)?)),
                "cleared" => "Signal cleared.".into(),
                "all_clear" => "All-clear. Every linked signal cleared.".into(),
                "transferred" => format!("Signal moved to incident {}.", r.get::<_, i64>(8)?),
                "continued" => format!("Continued in incident {}.", r.get::<_, i64>(8)?),
                "acknowledged" => "Acknowledged.".into(),
                "snoozed" => "Snoozed for one hour.".into(),
                _ => unreachable!("incident event kind checked by the store"),
            };
            Ok(Event { id: r.get(0)?, at: r.get(1)?, kind, actor: r.get(3)?, level, words })
        })
}

async fn timeline(State(app): State<Shared>, Path(id): Path<i64>, Query(cursor): Query<Cursor>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        Ok(page(events(tx, id, cursor.before)?, |event| event.id))
    })).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Press { action: Action }

async fn act(State(app): State<Shared>, Path(id): Path<i64>, Json(press): Json<Press>) -> Response {
    let result = store(&app, move |tx| {
        let changed = incidents::act(tx, id, press.action, now())?;
        let summary = tx.query_row(&format!("{SUMMARY} WHERE i.id = ?1"), [id], incident)
            .optional().map_err(sql)?.ok_or("Incident not found.")?;
        Ok((summary, changed))
    }).await;
    answer(result.map(|(summary, changed)| {
        if changed {
            app.supervisor.incident_changed();
        }
        summary
    }))
}
