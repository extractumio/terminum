// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Per-project live collection and independent heartbeat watchdog (AC-216 to AC-219).

use std::time::Duration;

use axum::{Json, Router, extract::{Path, State}, response::Response, routing::get};
use parking_lot::Mutex;
use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, watch};
use tokio::time::{MissedTickBehavior, interval};

use crate::app::Shared;
use crate::http::off;
use crate::tasks::{api::answer, ops::{check_project, now, rows, sql}, store::Store};

use super::{DAY, Evaluation, EvaluationInputs, Evidence, Mode, Reading, Rule, SCHEMA, Window,
    attach, evaluate_snapshot, flow, incidents, samples, sensors::{Pending, Sensors}, settings, settings_api, signals};

const BEAT: i64 = 10;
const DEAD_AFTER: i64 = 2 * BEAT;
const AGE_EVERY: i64 = 30;

#[derive(Default)]
pub(crate) struct Runtime {
    sensors: Sensors,
    // A scope refresh cannot overwrite a newer setting. Hook delivery never takes this lock.
    scopes: Mutex<()>,
    changed: Notify,
    pub(super) incident_updates: watch::Sender<u64>,
    pub(super) line_serial: tokio::sync::Mutex<()>,
}

#[derive(Clone)]
struct Project {
    id: String,
    generation: i64,
    enabled_at: i64,
    cursor: i64,
    last_ok: Option<i64>,
    last_evaluation: Option<i64>,
    last_sample: Option<i64>,
    last_baseline: Option<i64>,
    reported_dead: bool,
    settings_revision: Option<i64>,
}

const ENABLED: &str = "SELECT project, generation, enabled_at, cursor, last_ok, last_evaluation, last_sample, last_baseline, reported_dead, settings_revision
    FROM project_supervisor WHERE enabled = 1";

fn enabled(tx: &Transaction) -> Result<Vec<Project>, String> {
    rows(tx, ENABLED, [], |r| Ok(Project {
        id: r.get(0)?, generation: r.get(1)?, enabled_at: r.get(2)?, cursor: r.get(3)?,
        last_ok: r.get(4)?, last_evaluation: r.get(5)?, last_sample: r.get(6)?, last_baseline: r.get(7)?, reported_dead: r.get(8)?,
        settings_revision: r.get(9)?,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Switch { enabled: bool }

#[derive(Default, Serialize)]
struct Status {
    enabled: bool,
    last_ok: Option<i64>,
    collector_dead: bool,
    attachment: Option<serde_json::Value>,
}

pub(crate) fn routes() -> Router<Shared> {
    // Only the signed-in page router mounts this. The agent socket has no settings route.
    Router::new()
        .route("/api/task-projects/{id}/supervisor", get(shown).put(set))
        .merge(settings_api::routes())
        .merge(super::screen::routes())
        .merge(super::incidents_api::routes())
        .merge(super::rounds::routes())
        .merge(super::line_api::routes())
}

async fn shown(State(app): State<Shared>, Path(id): Path<String>) -> Response {
    answer(off(move || app.tasks.snapshot(SCHEMA, |tx| {
        check_project(tx, &id)?;
        let (mut status, current) = tx.query_row("SELECT enabled, last_ok, reported_dead,
                settings_revision IS coalesce((SELECT revision FROM supervisor_settings WHERE project = ?1), 0)
            FROM project_supervisor WHERE project = ?1", [&id],
            |r| Ok((Status { enabled: r.get(0)?, last_ok: r.get(1)?, collector_dead: r.get(2)?, attachment: None }, r.get::<_, bool>(3)?)))
            .optional().map_err(sql)?.unwrap_or_default();
        let active = status.enabled && current;
        status.attachment = attach::read(tx, &id)?.map(|report| report.status(now(), active)).transpose()?;
        Ok(status)
    })).await)
}

async fn set(State(app): State<Shared>, Path(id): Path<String>, Json(switch): Json<Switch>) -> Response {
    answer(off(move || {
        let runtime = &app.supervisor;
        let _scope = runtime.scopes.lock();
        if let Some(generation) = set_enabled(&app.tasks, &id, switch.enabled, now())? {
            runtime.sensors.set(&id, generation, switch.enabled);
            runtime.wake();
        }
        Ok(switch.enabled)
    }).await.map(|enabled| serde_json::json!({ "enabled": enabled })))
}

fn set_enabled(store: &Store, project: &str, on: bool, at: i64) -> Result<Option<i64>, String> {
    store.with(|tx| {
        check_project(tx, project)?;
        if on {
            tx.query_row("INSERT INTO project_supervisor (project, enabled, generation, enabled_at) VALUES (?1, 1, 1, ?2)
                ON CONFLICT (project) DO UPDATE SET enabled = 1, generation = generation + 1, enabled_at = excluded.enabled_at,
                    last_ok = NULL, reported_dead = 0 WHERE enabled = 0 RETURNING generation",
                (project, at), |r| r.get(0)).optional().map_err(sql)
        } else {
            tx.execute("DELETE FROM supervisor_round_requests WHERE fingerprint IN (
                SELECT fingerprint FROM supervisor_signals WHERE project = ?1)", [project]).map_err(sql)?;
            tx.query_row("UPDATE project_supervisor SET enabled = 0, generation = generation + 1, reported_dead = 0
                WHERE project = ?1 AND enabled = 1 RETURNING generation", [project], |r| r.get(0)).optional().map_err(sql)
        }
    })
}

pub(crate) fn hook(app: &Shared, key: &str, event: &serde_json::Value) {
    let sensors = &app.supervisor.sensors;
    if !sensors.any_on() {
        return;
    }
    let project = app.dispatcher.pane_project(key).or_else(|| {
        // Project moves hold this lock across their store change. An event during
        // that ambiguous interval must not be assigned to either project.
        let projects = app.projects.try_lock()?;
        projects.of(key).map(|p| p.id.clone())
    });
    if let Some(project) = project {
        sensors.heard(&project, key, event, now());
    }
}

impl Runtime {
    pub(super) fn incident_changed(&self) {
        self.incident_updates.send_modify(|revision| *revision = revision.wrapping_add(1));
    }

    pub(super) fn wake(&self) {
        self.changed.notify_one();
    }

    fn read_scopes(&self, store: &Store, at: i64) -> Result<Vec<Project>, String> {
        let _scope = self.scopes.lock();
        let projects = store.snapshot(SCHEMA, enabled)?;
        self.sensors.refresh(projects.iter().map(|p| (p.id.as_str(), p.generation)), at);
        Ok(projects)
    }
}

pub(crate) async fn collect(app: Shared) {
    let mut clock = interval(Duration::from_secs(BEAT as u64));
    clock.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = clock.tick() => {},
            _ = app.supervisor.changed.notified() => {},
        }
        let a = app.clone();
        if let Err(error) = off(move || {
            let projects = a.supervisor.read_scopes(&a.tasks, now())?;
            for project in projects {
                if let Err(error) = collect_project(&a, &project, now) {
                    tracing::warn!(event = "supervisor.collect", project = %project.id, %error);
                }
            }
            Ok(())
        }).await {
            tracing::warn!(event = "supervisor.scopes", %error);
        }
    }
}

struct Batch {
    at: i64,
    cursor: i64,
    evaluation: Option<Evaluation>,
    points: Vec<samples::Point>,
    sampled: bool,
    baseline: bool,
    settings_revision: i64,
    attachment: Option<String>,
    first_attachment: bool,
}

fn read_batch(
    tx: &Transaction, app: &Shared, project: &Project, at: i64,
) -> Result<Batch, String> {
    let cursor = tx.query_row("SELECT seq FROM supervisor_changes WHERE project = ?1 AND seq > ?2
        ORDER BY seq DESC LIMIT 1", (&project.id, project.cursor), |r| r.get(0))
        .optional().map_err(sql)?.unwrap_or(project.cursor);
    let sampled = project.last_sample.is_none_or(|last| at - last >= samples::CADENCE);
    let baseline = sampled && project.last_baseline.is_none_or(|last| at - last >= DAY);
    let points = if sampled { samples::measure(tx, app, &project.id, at, baseline)? } else { Vec::new() };
    let previous = project.last_evaluation.unwrap_or(0);
    let first_run = project.last_ok.is_none();
    let settings_revision = settings::revision(tx, &project.id)?;
    let scan_due = first_run || project.settings_revision != Some(settings_revision);
    let changed = cursor != project.cursor || sampled || scan_due;
    let samples_ready = project.last_sample.is_some_and(|last| last >= previous);
    let hooks_ready = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_hook_counts WHERE project = ?1 AND at >= ?2)",
        (&project.id, previous), |r| r.get::<_, bool>(0)).map_err(sql)?;
    let age_due = at - previous >= AGE_EVERY;
    let rules = Rule::ALL.iter().copied().filter(|rule| match rule {
        Rule::CollectorDead | Rule::LineDead => false,
        Rule::QueueGrowth | Rule::LimitNear | Rule::StalePlan => samples_ready || sampled || scan_due,
        Rule::Loop | Rule::Subagents => hooks_ready || sampled || scan_due,
        Rule::WaitHuman | Rule::Starved | Rule::UserBugAge => age_due || changed,
        _ => changed,
    }).collect::<Vec<_>>();
    let mut batch = Batch { at, cursor: project.cursor, evaluation: None, points, sampled, baseline,
        settings_revision, attachment: None, first_attachment: false };
    if rules.is_empty() || at <= previous {
        return Ok(batch);
    }
    let saved = settings::read(tx, &project.id)?;
    let previous_attach = attach::read(tx, &project.id)?;
    let detectors = rules.into_iter().map(|rule| {
        let mut detector = saved.settings.detector(rule);
        if let Some(report) = &previous_attach {
            detector.mode = report.mode(&detector);
        }
        if saved.settings.muted(rule, at) {
            detector.mode = Mode::Off;
        }
        detector
    }).collect::<Vec<_>>();
    let bounds = scan_due.then(|| flow::history_bounds(tx, &project.id, at)).transpose()?;
    let inputs = EvaluationInputs { first: bounds.map(|(first, _)| first), current_state: true,
        observe_thresholds: scan_due, ..EvaluationInputs::default() };
    let mut evaluation = evaluate_snapshot(tx, &project.id, at, &detectors, inputs)?;
    if let Some(bounds) = bounds {
        let report = attach::scan(tx, &mut evaluation, &saved.settings, &detectors, previous_attach.as_ref(), bounds)?;
        batch.attachment = Some(serde_json::to_string(&report).map_err(|_| "Could not encode Supervisor attachment.")?);
        batch.first_attachment = previous_attach.is_none();
    }
    batch.cursor = cursor;
    batch.evaluation = Some(evaluation);
    Ok(batch)
}

fn collect_project(app: &Shared, project: &Project, clock: impl Fn() -> i64) -> Result<(), String> {
    let batch = app.tasks.snapshot(SCHEMA, |tx| {
        let current = tx.query_row("SELECT EXISTS (SELECT 1 FROM project_supervisor
            WHERE project = ?1 AND enabled = 1 AND generation = ?2)",
            (&project.id, project.generation), |r| r.get::<_, bool>(0)).map_err(sql)?;
        let at = clock();
        // On a same-second re-enable the previous rule evaluation still owns
        // that timestamp. Leave the new scope due until it can record results.
        let due = project.last_ok.is_some() || project.last_evaluation.is_none_or(|last| at > last);
        (current && due).then(|| read_batch(tx, app, project, at)).transpose()
    })?;
    let Some(batch) = batch else { return Ok(()) };
    let pending = app.supervisor.sensors.take(&project.id, project.generation);
    let (committed, incidents_changed) = commit(&app.tasks, project, batch, &pending, clock)?;
    if committed {
        pending.committed();
    }
    if incidents_changed {
        app.supervisor.incident_changed();
    }
    Ok(())
}

fn commit(store: &Store, project: &Project, batch: Batch, pending: &Pending<'_>, clock: impl FnOnce() -> i64) -> Result<(bool, bool), String> {
    let signals = batch.evaluation.as_ref().map(signals::prepare).transpose()?;
    let counts = pending.encode()?;
    store.with(|tx| {
        let at = batch.at;
        let heartbeat = clock().max(at);
        let enabled = tx.execute("UPDATE project_supervisor SET cursor = ?3, last_ok = ?4, reported_dead = 0,
            last_evaluation = coalesce(?5, last_evaluation), last_sample = coalesce(?6, last_sample),
            last_baseline = coalesce(?7, last_baseline), attachment = coalesce(?10, attachment),
            settings_revision = CASE WHEN ?10 IS NULL THEN settings_revision ELSE ?11 END
            WHERE project = ?1 AND enabled = 1 AND generation = ?2 AND cursor = ?8 AND last_ok IS ?9
              AND coalesce((SELECT revision FROM supervisor_settings WHERE project = ?1), 0) = ?11",
            (&project.id, project.generation, batch.cursor, heartbeat, batch.evaluation.as_ref().map(|e| e.at),
                batch.sampled.then_some(at), batch.baseline.then_some(at), project.cursor, project.last_ok,
                batch.attachment, batch.settings_revision)).map_err(sql)?;
        if enabled == 0 {
            return Ok((false, false));
        }
        let before = incidents::revision(tx)?;
        if let Some(signals) = signals {
            signals.write(tx, batch.first_attachment)?;
        }
        samples::write(tx, &project.id, at, &batch.points)?;
        Pending::write(tx, &project.id, at, counts)?;
        tx.execute("DELETE FROM supervisor_changes WHERE seq IN (
            SELECT seq FROM supervisor_changes WHERE project = ?1 AND seq <= ?2 ORDER BY seq LIMIT 256)",
            (&project.id, batch.cursor)).map_err(sql)?;
        signals::resolve_rule(tx, &project.id, Rule::CollectorDead, heartbeat)?;
        Ok((true, before != incidents::revision(tx)?))
    })
}

pub(crate) async fn watch(app: Shared) {
    let mut clock = interval(Duration::from_secs(BEAT as u64));
    clock.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        clock.tick().await;
        if app.started.elapsed().as_secs() < DEAD_AFTER as u64 {
            continue;
        }
        let a = app.clone();
        match off(move || check_heartbeat(&a.tasks, now())).await {
            Ok(true) => app.supervisor.incident_changed(),
            Ok(false) => (),
            Err(error) => tracing::warn!(event = "supervisor.watchdog", %error),
        }
    }
}

fn check_heartbeat(store: &Store, at: i64) -> Result<bool, String> {
    let stale = store.snapshot(SCHEMA, |tx| {
        Ok(enabled(tx)?.into_iter().filter(|p| !p.reported_dead && at - p.last_ok.unwrap_or(p.enabled_at) >= DEAD_AFTER).collect::<Vec<_>>())
    })?;
    let mut changed = false;
    for project in stale {
        match report_dead(store, &project, at) {
            Ok(written) => changed |= written,
            Err(error) => tracing::warn!(event = "supervisor.watchdog", project = %project.id, %error),
        }
    }
    Ok(changed)
}

fn report_dead(store: &Store, project: &Project, at: i64) -> Result<bool, String> {
    let since = project.last_ok.unwrap_or(project.enabled_at);
    let rule = Rule::CollectorDead;
    let reading = Reading::new("project", &project.id, "missed_beats", ((at - since) / BEAT) as f64,
        vec![Evidence::SupervisorHeartbeat { project: project.id.clone(), last_ok: project.last_ok }])
        .comparison(super::Comparison::AtLeast);
    let Some(evaluation) = Evaluation::critical(&project.id, rule, reading, Window { before: since, start: since, end: at }) else {
        return Ok(false);
    };
    let prepared = signals::prepare(&evaluation)?;
    let written = store.with(|tx| {
        let stale = tx.execute("UPDATE project_supervisor SET reported_dead = 1 WHERE project = ?1
            AND enabled = 1 AND generation = ?2 AND last_ok IS ?3 AND reported_dead = 0",
            (&project.id, project.generation, project.last_ok)).map_err(sql)?;
        if stale != 0 {
            prepared.write(tx, false)?;
        }
        Ok(stale != 0)
    })?;
    if written {
        tracing::warn!(event = "supervisor.collector_dead", project = %project.id, last_ok = ?project.last_ok);
    }
    Ok(written)
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
