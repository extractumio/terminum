// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-244: a retained board task is the incident's conversation.

use rusqlite::{OptionalExtension, Transaction};
use sha2::{Digest, Sha256};

use crate::tasks::{comments, ops::{self, NewTask, rows, sql}};
use super::line_queue::{self, Kind};

pub(super) fn ensure(tx: &Transaction, project: &str, incident: i64, at: i64) -> Result<String, String> {
    if let Some(slug) = tx.query_row("SELECT t.slug FROM supervisor_incidents i
        JOIN tasks t ON t.id = i.task AND t.project = i.project WHERE i.id = ?1 AND i.project = ?2",
        (incident, project), |r| r.get(0)).optional().map_err(sql)? { return Ok(slug); }
    let by = crate::roles::supervisor::author(tx, project)?;
    let task = ops::create(tx, &by, NewTask { title: format!("Supervisor incident {incident}"),
        text: format!("Thread for Supervisor incident {incident}. The Supervisor page shows its timeline and actions."),
        project: Some(project.into()), severity: Some("P0".into()), ..NewTask::default() }, at)?;
    super::task_policy::mark_card(tx, &task)?;
    tx.execute("UPDATE supervisor_incidents SET task = ?3 WHERE id = ?1 AND project = ?2",
        (incident, project, &task.key)).map_err(sql)?;
    Ok(task.id)
}

pub(super) fn backfill(tx: &Transaction, project: &str, at: i64) -> Result<Vec<String>, String> {
    rows(tx, "SELECT id FROM supervisor_incidents WHERE project = ?1 AND task IS NULL
        AND (closed_at IS NULL OR acknowledged_at IS NULL) ORDER BY id LIMIT 20",
        [project], |r| r.get(0))?.into_iter().map(|id| ensure(tx, project, id, at)).collect()
}

/// This association is captured at launch, so a later incident cannot acquire an old conversation.
pub(super) fn current(tx: &Transaction, project: &str, fingerprint: &str) -> Result<Option<i64>, String> {
    tx.query_row("SELECT i.id FROM supervisor_incident_signals m JOIN supervisor_incidents i ON i.id = m.incident
        WHERE i.project = ?1 AND m.fingerprint = ?2", (project, fingerprint), |r| r.get(0)).optional().map_err(sql)
}

/// None is an ordinary task. Incidents without a live recipient return Some(None).
pub(crate) fn live_pane(tx: &Transaction, task: &str) -> Result<Option<Option<String>>, String> {
    tx.query_row(&format!("SELECT coalesce(
        (SELECT r.pane_key FROM supervisor_rounds q JOIN runs r ON r.id = q.run
          JOIN supervisor_panes m ON m.round = q.id AND m.pane_key = r.pane_key
          WHERE q.incident = i.id AND ({}) AND ({}) ORDER BY q.rowid DESC LIMIT 1),
        (SELECT r.pane_key FROM runs r JOIN run_tasks rt ON rt.run = r.id
          WHERE rt.task = t.id AND r.project = t.project AND r.pane_key = t.session_key
            AND r.started_by = 'human' AND r.started_how = 'run' AND ({})))
        FROM supervisor_incidents i JOIN tasks t ON t.id = i.task AND t.project = i.project WHERE i.task = ?1",
        super::rounds::ACTIVE, super::talk::LIVE, super::talk::LIVE),
        (task, ops::now()), |r| r.get(0)).optional().map_err(sql)
}

pub(super) fn anchor_live(tx: &Transaction, project: &str, fingerprint: &str, incident: i64, at: i64) -> Result<(), String> {
    tx.execute(&format!("UPDATE supervisor_rounds AS q SET incident = ?4
        WHERE q.project = ?1 AND q.fingerprint = ?3 AND q.incident IS NULL AND ({})", super::rounds::ACTIVE),
        (project, at, fingerprint, incident)).map(drop).map_err(sql)
}

pub(super) fn answer(tx: &Transaction, project: &str, run: &str, text: &str, at: i64) -> Result<String, String> {
    let incident = tx.query_row("SELECT incident FROM supervisor_rounds WHERE run = ?1 AND project = ?2
        UNION ALL SELECT incident FROM supervisor_talks WHERE run = ?1 AND project = ?2",
        (run, project), |r| r.get::<_, Option<i64>>(0)).optional().map_err(sql)?.flatten()
        .ok_or("This session is not linked to an incident thread.")?;
    let text = crate::plans::rules::clean(text);
    if text.is_empty() || text.chars().count() > 4_000 {
        return Err("An incident answer needs 1–4,000 characters.".into());
    }
    let task = ensure(tx, project, incident, at)?;
    let by = crate::roles::supervisor::author(tx, project)?;
    let key = ops::key_of(tx, &task)?;
    let identity = crate::auth::crypto::hex(&Sha256::digest(format!("{run}\n{text}")));
    if !comments::noticed(tx, &by, &key, "incident_answer", &identity)? {
        let event = comments::notice(tx, &by, &task, &text, serde_json::json!({"incident_answer":identity}), at)?;
        if let Some(s) = super::action_tasks::run_signal(tx, project, run)? {
            super::outcomes::record(tx, &s, super::outcomes::Kind::Nudge, &event, &key, "", at)?;
        } else {
            super::outcomes::unattributed(tx, project, super::outcomes::Kind::Nudge, &event, &key, at)?;
        }
        line_queue::enqueue(tx, incident, Kind::Answer, false)?;
    }
    Ok(task)
}

pub(super) fn rule(tx: &Transaction, incident: i64, reason: &str, at: i64) -> Result<String, String> {
    let project: String = tx.query_row("SELECT project FROM supervisor_incidents WHERE id = ?1",
        [incident], |r| r.get(0)).optional().map_err(sql)?.ok_or("Incident not found.")?;
    let reason = crate::plans::rules::clean(reason);
    if reason.is_empty() || reason.chars().count() > 1_000 {
        return Err("An owner ruling needs 1–1,000 characters.".into());
    }
    let task = ensure(tx, &project, incident, at)?;
    let event = comments::comment(tx, crate::tasks::store::HUMAN, &task, &format!("Owner ruling: {reason}"), at)?;
    tx.execute("INSERT OR IGNORE INTO supervisor_incident_rulings (incident, fingerprint, evidence, event)
        SELECT ?1, s.fingerprint, s.evidence, ?2 FROM supervisor_incident_signals i
        JOIN supervisor_signals s USING (fingerprint) WHERE i.incident = ?1",
        (incident, event)).map_err(sql)?;
    tx.execute("UPDATE supervisor_signals SET state = 'muted' WHERE project = ?2 AND fingerprint IN (
        SELECT fingerprint FROM supervisor_incident_rulings WHERE incident = ?1 AND evidence = supervisor_signals.evidence)
        AND state IN ('new','raised','recurred')", (incident, &project)).map_err(sql)?;
    super::incidents::act(tx, incident, super::incidents::Action::Acknowledge, at)?;
    Ok(task)
}

pub(super) fn clear(tx: &Transaction, incident: i64, at: i64) -> Result<(), String> {
    let task: Option<String> = tx.query_row("SELECT t.slug FROM supervisor_incidents i JOIN tasks t ON t.id = i.task AND t.project = i.project
        WHERE i.id = ?1 AND t.status = 'TODO' AND t.session_key IS NULL AND t.archived_at IS NULL
          AND NOT EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.author = 'human' AND e.kind = 'move')",
        [incident], |r| r.get(0)).optional().map_err(sql)?;
    if let Some(task) = task { ops::move_to(tx, crate::tasks::store::DISPATCHER, &task, "DONE", at)?; }
    Ok(())
}
