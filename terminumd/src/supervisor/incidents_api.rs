// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-224–226: incident reads and owner actions on the signed-in page listener.

use axum::{Json, Router, extract::{Path, Query, State, ws::{Message, WebSocket, WebSocketUpgrade}},
    http::HeaderMap, response::{IntoResponse, Response}, routing::{get, post}};
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
        .route("/api/supervisor/incidents/{id}/talk", post(talk))
        .route("/api/supervisor/incidents/{id}/ruling", post(ruling))
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
    task: Option<String>,
    held: String,
    holds: Vec<IncidentHold>,
    alerts: Vec<IncidentAlert>,
    needed: &'static str,
    no_answer: &'static str,
}

#[derive(Debug, Serialize)]
struct IncidentAlert { id: i64, detector: String, metric: String, response: String, wrong: bool }

#[derive(Debug, Serialize)]
struct IncidentHold { id: i64, label: &'static str, until: Option<i64> }

const SUMMARY: &str = "SELECT i.id, i.project, p.name, m.level,
      i.opened_at, i.closed_at, i.acknowledged_at, i.snoozed_until,
      (SELECT max(id) FROM supervisor_incident_events WHERE incident = i.id),
      m.metric, m.value, m.threshold,
      (SELECT count(*) FROM supervisor_incident_events WHERE incident = i.id AND kind = 'signal'),
      (SELECT related_id FROM supervisor_incident_events WHERE incident = i.id AND kind = 'continued'),
      (SELECT group_concat('Channel ' || channel || ': ' || failure, ' ' ORDER BY channel)
        FROM supervisor_line_failures WHERE i.subject_kind = 'critical_line' AND i.closed_at IS NULL),
      (SELECT t.slug FROM tasks t WHERE t.id = i.task AND t.project = i.project),
      (SELECT json_group_array(json_array(h.id, h.kind, h.until) ORDER BY h.id)
        FROM supervisor_holds h WHERE h.incident = i.id AND h.shadow = 0 AND h.released_at IS NULL),
      (SELECT json_group_array(json_array(id, detector, metric, response, wrong)) FROM (
        SELECT a.id, a.detector, e.metric, a.response,
            EXISTS (SELECT 1 FROM supervisor_outcomes o WHERE o.action = a.id AND o.days = 0) AS wrong
        FROM supervisor_actions a JOIN supervisor_incident_events e ON e.id = a.source
        WHERE a.project = i.project AND a.kind = 'alert' AND a.target = CAST(i.id AS TEXT)
          AND a.id = (SELECT max(other.id) FROM supervisor_actions other WHERE other.project = a.project
            AND other.kind = a.kind AND other.target = a.target AND other.fingerprint = a.fingerprint)
        ORDER BY a.id DESC LIMIT 50))
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
    let raw: Vec<(i64, String, Option<i64>)> = super::signals::decode(row, 16)?;
    let holds: Vec<_> = raw.into_iter().map(|(id, kind, until)| IncidentHold {
        id, label: super::holds::Kind::read(&kind).label(), until,
    }).collect();
    let raw: Vec<(i64, String, String, String, u8)> = super::signals::decode(row, 17)?;
    let alerts = raw.into_iter().map(|(id, detector, metric, response, wrong)|
        IncidentAlert { id, detector, metric, response, wrong: wrong != 0 }).collect();
    let held = if holds.is_empty() { "No hold in effect.".into() } else {
        holds.iter().map(|h| if h.until.is_some() { format!("{} while its verification runs.", h.label) }
            else { format!("{}.", h.label) }).collect::<Vec<_>>().join(" ")
    };
    // AC-225: the page shows each interim hold's deadline in local time; this sentence says what follows it.
    let interim_only = !holds.is_empty() && holds.iter().all(|h| h.until.is_some());
    Ok(Incident {
        id: row.get(0)?, project: row.get(1)?, project_name: row.get(2)?, level: row.get(3)?,
        opened_at: row.get(4)?, closed_at, acknowledged_at, snoozed_until: row.get(7)?, revision: row.get(8)?,
        happened: format!("{status}{measured} {count} {} recorded.{detail}", if count == 1 { "signal" } else { "signals" }),
        task: row.get(15)?,
        held, needed: if !holds.is_empty() { "Keep or release the hold." } else if acknowledged_at.is_some() { "Acknowledged." } else { "Acknowledge when seen." },
        no_answer: if holds.is_empty() { "No automatic action without an answer." }
            else if interim_only { "Without an answer the hold ends with its verification, unless verification confirms the signal." }
            else { "Without an answer the hold stays." },
        holds, alerts,
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
                "escalated" => format!("Level {}. A checked round escalated the signal. {}", level.unwrap_or(0),
                    measurement(&r.get::<_, String>(5)?, r.get(6)?, r.get(7)?)),
                "cleared" => "Signal cleared.".into(),
                "all_clear" => "All-clear. Every linked signal cleared.".into(),
                "transferred" => format!("Signal moved to incident {}.", r.get::<_, i64>(8)?),
                "continued" => format!("Continued in incident {}.", r.get::<_, i64>(8)?),
                "acknowledged" => "Acknowledged.".into(),
                "snoozed" => "Snoozed for one hour.".into(),
                "hold_applied" => format!("Hold {} applied.", r.get::<_, i64>(8)?),
                "hold_kept" => format!("Hold {} kept.", r.get::<_, i64>(8)?),
                "hold_released" => format!("Hold {} released.", r.get::<_, i64>(8)?),
                "hold_confirmed" => format!("Hold {} stays: verification confirmed its signal.", r.get::<_, i64>(8)?),
                "hold_end_failed" => format!("Hold {} could not end with its verification. Keep or release it.", r.get::<_, i64>(8)?),
                "alert_wrong" => "Owner marked the alert wrong.".into(),
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
#[serde(rename_all = "snake_case")]
enum Key { Acknowledge, Snooze, KeepHold, ReleaseHold, ThisIsWrong }

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Press { action: Key, hold: Option<i64>, alert: Option<i64> }

async fn owner_press(app: &Shared, id: i64, press: Press) -> Result<Incident, String> {
    let with_hold = matches!(press.action, Key::KeepHold | Key::ReleaseHold);
    if with_hold != press.hold.is_some() || matches!(press.action, Key::ThisIsWrong) != press.alert.is_some() {
        return Err("Choose the key and its hold or alert.".into());
    }
    if matches!(press.action, Key::ReleaseHold) {
        let project = store(app, move |tx| {
            tx.query_row("SELECT project FROM supervisor_incidents WHERE id = ?1", [id], |r| r.get::<_, String>(0))
                .optional().map_err(sql)?.ok_or_else(|| "Incident not found.".into())
        }).await?;
        super::holds_api::release(app, project, press.hold.unwrap(), None, Some(id)).await?;
    }
    let (summary, changed) = store(app, move |tx| {
        let at = now();
        let changed = match press.action {
            Key::Acknowledge => incidents::act(tx, id, Action::Acknowledge, at)?,
            Key::Snooze => incidents::act(tx, id, Action::Snooze, at)?,
            Key::KeepHold => {
                let hold = press.hold.unwrap();
                let active: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_holds
                    WHERE id = ?1 AND incident = ?2 AND shadow = 0 AND released_at IS NULL)",
                    (hold, id), |r| r.get(0)).map_err(sql)?;
                if !active { return Err("That hold is no longer in effect on this incident.".into()); }
                incidents::hold_event(tx, id, hold, "hold_kept", crate::tasks::store::HUMAN, at)?;
                true
            }
            Key::ThisIsWrong => incidents::wrong_alert(tx, id, press.alert.unwrap(), at)?,
            Key::ReleaseHold => false, // its transaction already sent the update
        };
        let summary = tx.query_row(&format!("{SUMMARY} WHERE i.id = ?1"), [id], incident)
            .optional().map_err(sql)?.ok_or("Incident not found.")?;
        Ok((summary, changed))
    }).await?;
    if changed { app.supervisor.incident_changed(); app.supervisor.wake(); }
    Ok(summary)
}

async fn act(State(app): State<Shared>, Path(id): Path<i64>, Json(press): Json<Press>) -> Response {
    answer(owner_press(&app, id, press).await)
}

async fn talk(State(app): State<Shared>, Path(id): Path<i64>) -> Response {
    answer(crate::dispatcher::supervisor_talk::open(&app, id).await)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Ruling { reason: String }

async fn ruling(State(app): State<Shared>, Path(id): Path<i64>, Json(input): Json<Ruling>) -> Response {
    let result = store(&app, move |tx| super::thread::rule(tx, id, &input.reason, now())).await;
    if let Ok(task) = &result {
        crate::tasks::api::told(&app, task);
        app.supervisor.incident_changed();
    }
    answer(result.map(|task| serde_json::json!({"task":task})))
}
