// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-246–247: keep the originating rule through task creation, queue promotion and Dispatcher admission.

use rusqlite::{OptionalExtension, Transaction};
use crate::tasks::ops::{Task, sql};
use super::{Signal, breakers, outcomes::{self, Kind}, signals};

pub(super) fn problem_signal(tx: &Transaction, project: &str, problem: i64) -> Result<Option<Signal>, String> {
    let key = tx.query_row("SELECT y.fingerprint FROM supervisor_problem_signals y
        JOIN supervisor_problems p ON p.id = y.problem JOIN supervisor_signals s USING (fingerprint)
        WHERE p.id = ?1 AND p.project = ?2 ORDER BY
          coalesce(max(json_extract(s.scores, '$.impact'),json_extract(s.scores, '$.blast'),
            json_extract(s.scores, '$.reversibility')),0) DESC, s.first_seen, s.fingerprint LIMIT 1",
        (problem, project), |r| r.get::<_, String>(0)).optional().map_err(sql)?;
    key.map(|key| signals::get(tx, project, &key)).transpose()
}

pub(super) fn run_signal(tx: &Transaction, project: &str, run: &str) -> Result<Option<Signal>, String> {
    let key = tx.query_row("SELECT fingerprint FROM supervisor_rounds WHERE project = ?1 AND run = ?2
        UNION ALL SELECT m.fingerprint FROM supervisor_talks t
          JOIN supervisor_incident_signals m ON m.incident = t.incident WHERE t.project = ?1 AND t.run = ?2
        LIMIT 1", (project, run), |r| r.get::<_, String>(0)).optional().map_err(sql)?;
    key.map(|key| signals::get(tx, project, &key)).transpose()
}

fn for_task(tx: &Transaction, project: &str, key: &str, linked: bool) -> Result<Option<(Signal, String)>, String> {
    if let Some(a) = outcomes::find(tx, project, Kind::Fix, key)? {
        return Ok(Some((signals::get(tx, project, &a.fingerprint)?, a.response)));
    }
    if !linked { return Ok(None); }
    let Some(problem) = super::register::active_for_task(tx, project, key)?.first().copied() else { return Ok(None); };
    Ok(problem_signal(tx, project, problem)?.map(|s| { let response = s.response.into(); (s, response) }))
}

fn origin(tx: &Transaction, project: &str, key: &str, pane: Option<&str>) -> Result<Option<(Signal, String)>, String> {
    if let Some(known) = for_task(tx, project, key, pane.is_some())? { return Ok(Some(known)); }
    let Some(pane) = pane else { return Ok(None) };
    let Some(mark) = crate::roles::supervisor::marked(tx, pane)? else { return Ok(None) };
    if mark.project != project { return Err("The task is outside this project.".into()); }
    let Some(run) = mark.run else { return Ok(None) };
    Ok(run_signal(tx, project, &run)?.map(|s| { let response = s.response.into(); (s, response) }))
}

pub(crate) fn divert(tx: &Transaction, project: &str, key: &str, id: &str, pane: Option<&str>,
    kind: &str, at: i64) -> Result<Option<breakers::Card>, String>
{
    if pane.is_none() && super::rounds::enabled(tx, project)?.is_none() { return Ok(None); }
    let Some((s, response)) = origin(tx, project, key, pane)? else { return Ok(None) };
    let words = if kind == "promote" { format!("The Supervisor would promote {id} in the run queue.") }
        else { format!("The Supervisor would start {id}.") };
    breakers::divert_rule(tx, &s, &response, kind, key, &words, at)
}

pub(crate) fn record_run(tx: &Transaction, task: &Task, pane: Option<&str>, source: &str, run: &str, at: i64)
    -> Result<(), String>
{
    if pane.is_none() && super::rounds::enabled(tx, &task.project)?.is_none() { return Ok(()); }
    if let Some((s, response)) = origin(tx, &task.project, &task.key, pane)? {
        outcomes::record_rule(tx, (&s, &response), Kind::Run, source, &task.key, run, at)?;
    }
    Ok(())
}

pub(crate) fn comment(tx: &Transaction, task: &Task, pane: &str, event: &str, question: bool, at: i64) -> Result<(), String> {
    let kind = if question { Kind::Card } else { Kind::Nudge };
    if let Some((s, response)) = origin(tx, &task.project, &task.key, Some(pane))? {
        return outcomes::record_rule(tx, (&s, &response), kind, event, &task.key, "", at);
    }
    // Comments without an attributable detector still have a retained decision.
    outcomes::unattributed(tx, &task.project, kind, event, &task.key, at)
}

pub(crate) fn divert_group(tx: &Transaction, tasks: &[Task], pane: Option<&str>, at: i64)
    -> Result<Vec<String>, String>
{
    let mut cards = Vec::new();
    if let [lead, ..] = tasks && !has_breaker(tx, &lead.project)? { return Ok(cards); }
    for task in tasks {
        if let Some(card) = divert(tx, &task.project, &task.key, &task.id, pane, "run", at)? {
            cards.push(card.task);
        }
    }
    Ok(cards)
}

pub(crate) fn diverted_hold_run(tx: &Transaction, project: &str, id: i64, at: i64) -> Result<Option<breakers::Card>, String> {
    let held = tx.query_row("SELECT h.fingerprint, h.response, h.target FROM supervisor_holds h
        JOIN supervisor_breakers b ON b.project = h.project AND b.detector = h.detector AND b.response = h.response
        WHERE h.project = ?1 AND h.id = ?2 AND h.released_at IS NULL AND h.kind = 'stop_run' AND b.closed_at IS NULL",
        (project, id), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
        .optional().map_err(sql)?;
    let Some((fingerprint, response, target)) = held else { return Ok(None) };
    let signal = signals::get(tx, project, &fingerprint)?;
    breakers::divert_rule(tx, &signal, &response, "resume_run", &target, "The Supervisor would restart the held run.", at)
}

pub(crate) fn has_breaker(tx: &Transaction, project: &str) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_breakers b JOIN project_supervisor s USING (project)
        WHERE b.project = ?1 AND b.closed_at IS NULL AND s.enabled)",
        [project], |r| r.get(0)).map_err(sql)
}
