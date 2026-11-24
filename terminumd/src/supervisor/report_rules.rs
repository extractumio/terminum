// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Reports as the Supervisor's evidence (terminum-reports.md AC-276, AC-277): a critical report is a critical
//! signal of its subject, opening or joining that subject's incident at once; the owner's unread reports and
//! unanswered replies are readings of their own.

use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::ops::{rows, sql};

use super::{Evaluation, Evidence, Measurements, Query, Reading, Rule, Window, impact::Source, rounds, signals};

/// A critical report's level is its author's, not a share of the project's week.
const DECLARED: Source = Source::Unsized("A critical report is its author's declaration, in no weekly project total.");
const READ_STATE: Source = Source::Unsized("A report's read state is in no weekly project total.");
const METRIC: &str = "critical_reports";

/// A critical report alerts once (AC-276): it counts while unread, not an incident's or a Supervisor round's own,
/// not alerted by its own line, not answered by the owner's reply, and its incident, if any, still open.
const ALERTING: &str = "critical AND incident IS NULL AND role <> 'supervisor' AND line = 0 AND replied_at IS NULL
    AND read_at IS NULL AND deleted_at IS NULL
    AND (alerted_by IS NULL OR alerted_by IN (SELECT id FROM supervisor_incidents WHERE closed_at IS NULL))";

/// The reports alerting, one reading per subject: the task they were made for, else the report itself; each lists
/// every such report as evidence. Present only: reading or deleting them clears the signal, and with it the incident.
pub(super) fn critical(q: &Query<'_, '_>) -> Result<Measurements, String> {
    Ok(Measurements { readings: subjects(q.tx, q.project, None)?, ..Measurements::default() })
}

fn subjects(tx: &Transaction, project: &str, only: Option<&str>) -> Result<Vec<Reading>, String> {
    let found = rows(tx, &format!("SELECT task IS NULL, coalesce(task, id), id FROM reports
        WHERE project = ?1 AND {ALERTING} AND (?2 IS NULL OR coalesce(task, id) = ?2) ORDER BY 2, created_at, id"),
        (project, only), |r| Ok((r.get::<_, bool>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut readings: Vec<Reading> = Vec::new();
    for (alone, subject, key) in found {
        match readings.last_mut() {
            Some(last) if last.subject_id == subject => { last.evidence.push(Evidence::Reports(key)); last.value += 1.0; }
            _ => readings.push(Reading::new(if alone { "report" } else { "task" }, subject, METRIC, 1.0, vec![Evidence::Reports(key)], DECLARED)),
        }
    }
    Ok(readings)
}

/// The incident a critical signal opened or joined now alerts for each of its reports (`alerted_by`), a newer one
/// taking the subject's reports over from an older (AC-276); called by the incident's own transition, on every path.
pub(super) fn alerted(tx: &Transaction, incident: i64, evidence: &[Evidence]) -> Result<(), String> {
    for key in evidence.iter().filter_map(|e| match e { Evidence::Reports(key) => Some(key), _ => None }) {
        tx.execute("UPDATE reports SET alerted_by = ?1 WHERE id = ?2", (incident, key)).map_err(sql)?;
    }
    Ok(())
}

/// Report `key` was made or turned critical, or not: with its project's Supervisor on, its subject is measured
/// now, as the collector would, limited to that subject. Answers the incident alerting for it; none (Supervisor
/// off, the detector ruled wrong, a breaker open) leaves the report's own line to alert.
pub(crate) fn supervise(tx: &Transaction, key: &str, at: i64) -> Result<Option<i64>, String> {
    let Some((project, subject)) = tx.query_row("SELECT project, coalesce(task, id) FROM reports WHERE id = ?1", [key],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional().map_err(sql)? else { return Ok(None) };
    if rounds::enabled(tx, &project)?.is_none() { return Ok(None); }
    let rule = Rule::ReportCritical;
    let kind = if subject == key { "report" } else { "task" };
    let at = signals::after_last(tx, &project, rule, at)?;
    let window = Window { before: at, start: at, end: at };
    if let Some(evaluation) = Evaluation::critical(&project, rule, subjects(tx, &project, Some(&subject))?,
        Some(vec![(kind, subject.clone(), METRIC)]), window)
    {
        signals::prepare(&evaluation)?.write(tx, false)?;
    }
    tx.query_row("SELECT alerted_by FROM reports WHERE id = ?1 AND alerted_by IN (SELECT id FROM supervisor_incidents WHERE closed_at IS NULL)",
        [key], |r| r.get(0)).optional().map_err(sql)
}

/// The owner's oldest unread report, one reading per project, in minutes (AC-277).
pub(super) fn unread_age(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let unread = rows(q.tx, "SELECT id, created_at FROM reports WHERE project = ?1 AND read_at IS NULL AND deleted_at IS NULL
        AND incident IS NULL ORDER BY created_at, id LIMIT 20", [q.project], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let Some(&(_, oldest)) = unread.first() else { return Ok(Measurements::default()) };
    let evidence = unread.into_iter().map(|(key, _)| Evidence::Reports(key)).collect();
    let reading = Reading::new("project", q.project, "unread_reports", ((q.window.end - oldest) as f64 / 60.0).max(0.0), evidence, READ_STATE);
    Ok(Measurements { readings: vec![reading], ..Measurements::default() })
}

/// Replies the owner sent that no agent has answered yet, each in minutes since it was sent (AC-277): no agent
/// comment after it, no follow-up report (as reply.rs's answer state reads them).
pub(super) fn follow_up_age(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let waiting = rows(q.tx, &format!("SELECT p.id, p.replied_at FROM reports p WHERE p.project = ?1 AND p.replied_at IS NOT NULL
        AND p.deleted_at IS NULL AND {} ORDER BY p.replied_at, p.id", crate::reports::reply::UNANSWERED), [q.project], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let readings = waiting.into_iter().map(|(key, sent)| Reading::new("report", key.clone(), "unanswered_replies",
        ((q.window.end - sent) as f64 / 60.0).max(0.0), vec![Evidence::Reports(key)], READ_STATE)).collect();
    Ok(Measurements { readings, ..Measurements::default() })
}
