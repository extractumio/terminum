// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Reports as the Supervisor's evidence (terminum-reports.md AC-276, AC-277): a critical report is a critical
//! signal of its subject, opening or joining that subject's incident at once; the owner's unread reports and
//! unanswered replies are readings of their own.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction};

use crate::tasks::ops::{rows, sql};

use super::{Checked, Evaluation, Evidence, Measurements, Mode, Query, Reading, Rule, Scores, Window, impact::Source, rounds, signals};

/// A critical report's level is its author's, not a share of the project's week.
const DECLARED: Source = Source::Unsized("A critical report is its author's declaration, in no weekly project total.");
const READ_STATE: Source = Source::Unsized("A report's read state is in no weekly project total.");
const METRIC: &str = "critical_reports";

/// The unread critical reports not an incident's own, one reading per subject: the task they were made for,
/// else the report itself; each lists every such report as evidence. Present only: reading or deleting them
/// clears the signal, and with it the incident (AC-276).
pub(super) fn critical(q: &Query<'_, '_>) -> Result<Measurements, String> {
    Ok(Measurements { readings: subjects(q.tx, q.project, None)?, ..Measurements::default() })
}

fn subjects(tx: &Transaction, project: &str, only: Option<(&str, &str)>) -> Result<Vec<Reading>, String> {
    let found = rows(tx, "SELECT CASE WHEN task IS NULL THEN 'report' ELSE 'task' END, coalesce(task, id), id FROM reports
        WHERE project = ?1 AND critical AND incident IS NULL AND read_at IS NULL AND deleted_at IS NULL
          AND (?2 IS NULL OR (CASE WHEN task IS NULL THEN 'report' ELSE 'task' END = ?2 AND coalesce(task, id) = ?3))
        ORDER BY 1, 2, created_at, id", (project, only.map(|o| o.0), only.map(|o| o.1)),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut by: BTreeMap<(String, String), Vec<Evidence>> = BTreeMap::new();
    for (kind, subject, key) in found {
        by.entry((kind, subject)).or_default().push(Evidence::Reports(key));
    }
    Ok(by.into_iter().map(|((kind, subject), evidence)| {
        Reading::new(if kind == "task" { "task" } else { "report" }, subject, METRIC, evidence.len() as f64, evidence, DECLARED)
    }).collect())
}

/// Report `key` was made or turned critical: with its project's Supervisor on, its subject is measured now, as
/// the collector would, limited to that subject. Answers the incident whose alert stands for it, which every
/// unread report of that subject then names (`alerted_by`); none (Supervisor off, the detector muted, a breaker
/// open) leaves the report's own line to alert.
pub(crate) fn supervise(tx: &Transaction, key: &str, at: i64) -> Result<Option<i64>, String> {
    let Some((project, task)) = tx.query_row("SELECT project, task FROM reports WHERE id = ?1", [key],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).optional().map_err(sql)? else { return Ok(None) };
    if rounds::enabled(tx, &project)?.is_none() { return Ok(None); }
    let subject = match &task { Some(task) => ("task", task.as_str()), None => ("report", key) };
    let rule = Rule::ReportCritical;
    // two reports in one second are two evaluations: each must advance the detector's clock (AC-212)
    let last = tx.query_row("SELECT evaluated_at FROM supervisor_evaluations WHERE project = ?1 AND detector = ?2",
        (&project, rule.id()), |r| r.get::<_, i64>(0)).optional().map_err(sql)?;
    let at = last.map_or(at, |last| at.max(last + 1));
    let window = Window { before: at, start: at, end: at };
    let findings = subjects(tx, &project, Some(subject))?.into_iter()
        .filter_map(|reading| reading.finding(rule.defaults(), window))
        .map(|mut found| { found.scores = Scores { impact: rule.fixed_impact().unwrap_or(3), ..Scores::default() }; found }).collect();
    let evaluation = Evaluation { project: project.clone(), at, detectors: vec![Checked {
        rule, mode: Mode::Live, unavailable: Vec::new(), findings, thresholds: Default::default(),
        complete_metrics: Vec::new(), measured: Some(vec![(subject.0, subject.1.to_string(), METRIC)]),
        observations: Default::default(),
    }] };
    let before: i64 = tx.query_row("SELECT coalesce(max(id), 0) FROM supervisor_incident_events", [], |r| r.get(0)).map_err(sql)?;
    signals::prepare(&evaluation)?.write(tx, false)?;
    let fingerprint = signals::fingerprint(&project, rule.id(), (subject.0, subject.1, METRIC));
    let incident = tx.query_row("SELECT incident FROM supervisor_incident_events
        WHERE id > ?1 AND fingerprint = ?2 AND kind IN ('signal', 'updated') ORDER BY id DESC LIMIT 1",
        (before, &fingerprint), |r| r.get::<_, i64>(0)).optional().map_err(sql)?;
    if let Some(incident) = incident {
        // a newer incident taking the subject over takes its unread reports with it
        tx.execute("UPDATE reports SET alerted_by = ?1 WHERE project = ?2 AND critical AND incident IS NULL
            AND read_at IS NULL AND deleted_at IS NULL AND coalesce(task, id) = ?3", (incident, &project, subject.1)).map_err(sql)?;
    }
    Ok(incident)
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
