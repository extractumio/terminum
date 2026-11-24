// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-233: a low-confidence verification compares the modeled cost of waiting with the cost of the one interim
//! hold code may apply, taking the signal's own queued task out of integration, and ends that hold with its round
//! unless code confirms the signal.
//!
//! Both costs are weekly shares, the common currency of AC-213: waiting lets the signal take its measured share of
//! its quantity, discounted by its confidence; the hold stalls one task of the tasks the queue integrated in the
//! last seven days. The verification window multiplies both, so it cancels.

use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::Value;

use crate::tasks::ops::sql;

use super::{Evidence, Signal, WEEK, holds::{self, Kind, Request}, signals::{self, State}};

#[derive(Debug, Default, Serialize)]
pub(super) struct Comparison {
    /// The cost of waiting: the signal's latest weekly share times its confidence.
    pub wait_share: Option<f64>,
    /// The cost of holding: the share of the last seven days' integrated tasks the take-out stalls.
    pub hold_share: Option<f64>,
    /// The held task, as the board names it.
    pub task: Option<String>,
    pub hold: Option<i64>,
    pub decision: String,
    /// The applied hold's comments, task and event, for their notices (AC-241).
    #[serde(skip)]
    pub nudges: Vec<(String, String)>,
}

impl Comparison {
    fn decided(self, why: impl Into<String>) -> Self {
        Self { decision: why.into(), ..self }
    }
}

/// The signal's own task, then the tasks its evidence names, that wait in an active queue outside a batch.
fn queued_task(tx: &Transaction, signal: &Signal) -> Result<Option<String>, String> {
    let mut named = Vec::new();
    if signal.subject_kind == "task" { named.push(signal.subject_id.as_str()); }
    named.extend(signal.evidence.iter().filter_map(|e| match e { Evidence::Tasks(id) => Some(id.as_str()), _ => None }));
    let named = serde_json::to_string(&named).map_err(|e| e.to_string())?;
    tx.query_row("SELECT coalesce(t.slug, CAST(t.number AS TEXT)) FROM json_each(?2) n
        JOIN tasks t ON t.project = ?1 AND (t.id = n.value OR t.slug = n.value)
        JOIN integration_entries e ON e.task = t.id AND e.batch IS NULL
        JOIN project_integration s ON s.project = t.project AND s.is_on
        ORDER BY n.key LIMIT 1", (&signal.project, named), |r| r.get(0)).optional().map_err(sql)
}

fn integrated_last_week(tx: &Transaction, project: &str, at: i64) -> Result<i64, String> {
    tx.query_row("SELECT coalesce(sum(json_array_length(r.tasks)), 0) FROM integration_rounds r
        JOIN integration_batches b ON b.id = r.batch
        WHERE b.project = ?1 AND r.result = 'green' AND r.ended > ?2", (project, at - WEEK), |r| r.get(0)).map_err(sql)
}

/// Compare, and apply the interim hold when it costs less than waiting. A refused hold changes nothing.
pub(super) fn verification_hold(tx: &Transaction, signal: &Signal, round: &str, until: i64, at: i64) -> Result<Comparison, String> {
    let Some(share) = signal.impact_basis.latest_share() else {
        let why = signal.impact_basis.no_share.as_deref().unwrap_or("The signal has no measured share of a weekly total.");
        return Ok(Comparison::default().decided(format!("No interim hold: waiting has no modeled cost. {why}")));
    };
    let wait = share * signal.scores.confidence;
    let found = Comparison { wait_share: Some(wait), ..Comparison::default() };
    let Some(task) = queued_task(tx, signal)? else {
        return Ok(found.decided("No interim hold: the signal names no task waiting in the integration queue."));
    };
    let integrated = integrated_last_week(tx, &signal.project, at)?;
    if integrated == 0 {
        return Ok(found.decided("No interim hold: no task was integrated in the last seven days, so holding has no modeled cost."));
    }
    let hold = 1.0 / integrated as f64;
    let found = Comparison { hold_share: Some(hold), ..found };
    if hold >= wait {
        return Ok(found.decided("No interim hold: waiting costs less than holding."));
    }
    tx.execute_batch("SAVEPOINT interim_hold").map_err(sql)?;
    let request = Request { signal: signal.fingerprint.clone(), kind: Kind::TakeOut, task: Some(task.clone()),
        reason: "Interim hold while its verification runs.".into() };
    let applied = match holds::apply_interim(tx, &signal.project, request, at) {
        Ok(applied) => applied,
        Err(why) => {
            tx.execute_batch("ROLLBACK TO interim_hold; RELEASE interim_hold").map_err(sql)?;
            return Ok(found.decided(format!("No interim hold: {why}")));
        }
    };
    tx.execute_batch("RELEASE interim_hold").map_err(sql)?;
    let found = Comparison { hold: Some(applied.id), ..found };
    if applied.shadow {
        return Ok(found.decided("Holding costs less than waiting; the detector is in shadow, so the hold is recorded without effect."));
    }
    tx.execute("UPDATE supervisor_holds SET round = ?2, until = ?3 WHERE id = ?1", (applied.id, round, until)).map_err(sql)?;
    Ok(Comparison { task: Some(task), nudges: applied.nudges, ..found }
        .decided("Holding costs less than waiting: the task is out of integration until its verification ends."))
}

/// Whether code confirms the verified signal: a checked finding at strong confidence, the signal still current at
/// level 2 or above, and its measurement recomputed now still crossing its threshold.
fn confirmed(tx: &Transaction, project: &str, fingerprint: &str, case: &Value, finding: Option<&str>) -> Result<bool, String> {
    let confident = finding.and_then(|f| serde_json::from_str::<super::round_tools::Finding>(f).ok())
        .is_some_and(|f| f.confidence >= super::STRONG);
    if !confident { return Ok(false); }
    let signal = signals::get(tx, project, fingerprint)?;
    if signal.level < 2 || !matches!(signal.state, State::New | State::Raised | State::Recurred | State::Held) {
        return Ok(false);
    }
    Ok(super::round_tools::metric(tx, project, case, &signal.detector, fingerprint).is_ok_and(|r| r.fires))
}

/// At its round's end, keep a confirmed interim hold as an ordinary one and release any other. A release that fails
/// leaves an ordinary hold that says why, with its keys. Returns the task to tell.
pub(crate) fn round_ended(tx: &Transaction, run: &str, at: i64) -> Result<Option<String>, String> {
    let row = tx.query_row("SELECT h.id, h.project, h.fingerprint, r.case_file, r.finding FROM supervisor_holds h
        JOIN supervisor_rounds r ON r.id = h.round
        WHERE r.run = ?1 AND h.until IS NOT NULL AND h.shadow = 0 AND h.released_at IS NULL", [run],
        |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?))).optional().map_err(sql)?;
    let Some((id, project, fingerprint, case, finding)) = row else { return Ok(None) };
    let case: Value = serde_json::from_str(&case).map_err(|e| e.to_string())?;
    let hold = holds::get(tx, &project, id)?;
    let (event, failure) = if confirmed(tx, &project, &fingerprint, &case, finding.as_deref())? {
        ("hold_confirmed", None)
    } else {
        tx.execute_batch("SAVEPOINT interim_end").map_err(sql)?;
        match holds::release(tx, &project, &hold, "system", at) {
            Ok(()) => {
                tx.execute_batch("RELEASE interim_end").map_err(sql)?;
                return Ok(hold.task);
            }
            Err(why) => {
                tx.execute_batch("ROLLBACK TO interim_end; RELEASE interim_end").map_err(sql)?;
                ("hold_end_failed", Some(why))
            }
        }
    };
    tx.execute("UPDATE supervisor_holds SET until = NULL, end_failed = ?2 WHERE id = ?1", (id, failure)).map_err(sql)?;
    if let Some(incident) = hold.incident {
        super::incidents::hold_event(tx, incident, id, event, "system", at)?;
    }
    Ok(hold.task)
}
