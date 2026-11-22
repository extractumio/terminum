// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-224–226: incident reads and owner actions on the signed-in page listener.

use axum::{Json, Router, extract::{Path, Query, State, ws::{Message, WebSocket, WebSocketUpgrade}},
    http::HeaderMap, response::{IntoResponse, Response}, routing::get};
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
      (SELECT related_id FROM supervisor_incident_events WHERE incident = i.id AND kind = 'continued'),
      (SELECT group_concat('Channel ' || channel || ': ' || failure, ' ' ORDER BY channel)
        FROM supervisor_line_failures WHERE i.subject_kind = 'critical_line' AND i.closed_at IS NULL)
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
    let failure: Option<String> = row.get(14)?;
    let detail = failure.map_or_else(String::new, |status| format!(" {status}"));
    let status = match (closed_at, continued) {
        (_, Some(id)) => format!("Continued in incident {id}. "),
        (Some(_), None) => "All-clear. ".into(),
        (None, None) => String::new(),
    };
    Ok(Incident {
        id: row.get(0)?, project: row.get(1)?, project_name: row.get(2)?, level: row.get(3)?,
        opened_at: row.get(4)?, closed_at, acknowledged_at, snoozed_until: row.get(7)?, revision: row.get(8)?,
        happened: format!("{status}{measured} {count} {} recorded.{detail}", if count == 1 { "signal" } else { "signals" }),
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

// The page router has checked the sign-in and Origin before this upgrade. The socket
// only says a change occurred. The feed and every owner action check sign-in afresh.
async fn watch(State(app): State<Shared>, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    let credential = crate::auth::checks::credential_of(&headers).unwrap_or_default();
    let ws = ws.protocols(["tm"]).max_frame_size(1024).max_message_size(1024);
    ws.on_upgrade(move |socket| wakeups(app, credential, socket)).into_response()
}

async fn wakeups(app: Shared, credential: String, mut socket: WebSocket) {
    let mut revoked = app.signins.revocations();
    let mut updates = app.supervisor.incident_updates.subscribe();
    let signed_in = || app.signins.check(&credential, true);
    if !signed_in() {
        return;
    }
    let mut ping = tokio::time::interval(Duration::from_secs(15));
    loop {
        let outgoing = tokio::select! {
            result = revoked.changed() => {
                if result.is_err() || !signed_in() {
                    break;
                }
                continue;
            }
            _ = ping.tick() => {
                if !signed_in() {
                    break;
                }
                Message::Ping(Default::default())
            }
            result = updates.changed() => {
                if result.is_err() {
                    break;
                }
                Message::Text("changed".into())
            }
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => break,
            }
        };
        if !matches!(tokio::time::timeout(Duration::from_secs(10), socket.send(outgoing)).await, Ok(Ok(()))) {
            break;
        }
    }
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
