// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-257: inactivity changes delivery only and never removes a rule.

use std::collections::BTreeSet;

use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;

use crate::tasks::ops::{rows, sql};

use super::{Mode, Rule, WEEK, settings::Settings};

pub(super) fn protected(tx: &Transaction, project: &str, rule: Rule, settings: &Settings) -> Result<bool, String> {
    // A detector the owner tied to a never line keeps the owner's coverage: only the owner turns it down (AC-213).
    if !rule.configurable() || settings.detector(rule).never.is_some() { return Ok(true); }
    let protected: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_signals s
        WHERE s.project = ?1 AND s.detector = ?2 AND (
            max(json_extract(s.scores, '$.impact'), json_extract(s.scores, '$.blast'),
                json_extract(s.scores, '$.reversibility')) >= 3 OR EXISTS (
                SELECT 1 FROM supervisor_design_gates g WHERE g.project = s.project
                AND g.task = s.subject_id AND g.tier = 3 AND s.subject_kind = 'task')))
        OR EXISTS (SELECT 1 FROM supervisor_incident_events e JOIN supervisor_incidents i ON i.id = e.incident
            WHERE i.project = ?1 AND e.detector = ?2 AND e.level = 3)",
        (project, rule.id()), |r| r.get(0)).map_err(sql)?;
    if protected { return Ok(true); }
    let map = &settings.charter.surfaces;
    if map.t3_terms.is_empty() { return Ok(false); }
    let subjects = rows(tx, "SELECT DISTINCT t.title, t.body FROM supervisor_signals s JOIN tasks t ON t.id = s.subject_id
        WHERE s.project = ?1 AND s.detector = ?2 AND s.subject_kind = 'task' AND t.project = s.project",
        (project, rule.id()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    Ok(subjects.iter().any(|(title, text)| map.text_match(title, text).is_some()))
}

pub(super) fn mode_changed(tx: &Transaction, project: &str, rule: Rule, mode: Mode, at: i64) -> Result<(), String> {
    tx.execute("DELETE FROM supervisor_detector_lifecycle WHERE project = ?1 AND detector = ?2",
        (project, rule.id())).map_err(sql)?;
    if mode == Mode::Live {
        tx.execute("INSERT INTO supervisor_detector_lifecycle (project, detector, live_since) VALUES (?1, ?2, ?3)",
            (project, rule.id(), at)).map_err(sql)?;
    }
    if mode != Mode::Live { quiet_signals(tx, project, rule)?; }
    Ok(())
}

pub(super) fn quiet_signals(tx: &Transaction, project: &str, rule: Rule) -> Result<(), String> {
    tx.execute("UPDATE supervisor_signals SET state = 'shadow'
        WHERE project = ?1 AND detector = ?2 AND state IN ('new','raised','recurred')",
        (project, rule.id())).map(drop).map_err(sql)
}

pub(super) fn on_weekly(tx: &Transaction, project: &str) -> Result<BTreeSet<String>, String> {
    rows(tx, "SELECT detector FROM supervisor_detector_lifecycle WHERE project = ?1 AND weekly_at IS NOT NULL",
        [project], |r| r.get(0)).map(|rows| rows.into_iter().collect())
}

fn acted_on(tx: &Transaction, project: &str, rule: Rule, since: i64) -> Result<Option<i64>, String> {
    tx.query_row("SELECT max(at) FROM (
        SELECT v.at FROM supervisor_verdicts v JOIN supervisor_signals s USING(fingerprint)
        JOIN supervisor_review_reports r ON r.task = v.task AND r.project = s.project
        WHERE r.at >= ?3 AND s.project = ?1 AND s.detector = ?2 AND v.at >= ?3 AND v.verdict = 'useful'
            AND v.id = (SELECT max(w.id) FROM supervisor_verdicts w WHERE w.task = v.task AND w.fingerprint = v.fingerprint)
        UNION ALL SELECT a.at FROM supervisor_actions a WHERE a.project = ?1 AND a.detector = ?2 AND a.at >= ?3
            AND (a.kind IN ('run','hold') OR EXISTS (SELECT 1 FROM supervisor_outcomes o
                WHERE o.action = a.id AND o.verdict = 'right'))
        )", (project, rule.id(), since), |r| r.get(0)).map_err(sql)
}

pub(super) fn advance(tx: &Transaction, project: &str, enabled_at: i64, at: i64) -> Result<(), String> {
    if at - enabled_at < 8 * WEEK { return Ok(()); }
    let settings = super::settings::read(tx, project)?.settings;
    let report = super::attach::read(tx, project)?;
    for &rule in Rule::ALL {
        let detector = settings.detector(rule);
        if !rule.configurable() || detector.mode != Mode::Live ||
            report.as_ref().is_some_and(|r| r.mode(&detector, &settings) != Mode::Live) { continue; }
        let (since, weekly, removal): (i64, Option<i64>, Option<i64>) = tx.query_row(
            "SELECT live_since, weekly_at, removal_at FROM supervisor_detector_lifecycle WHERE project = ?1 AND detector = ?2",
            (project, rule.id()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).optional().map_err(sql)?
            .unwrap_or((enabled_at, None, None));
        if at - since.max(enabled_at) < 8 * WEEK { continue; }
        let idle = at - acted_on(tx, project, rule, at - 16 * WEEK)?.unwrap_or(since).max(since).max(enabled_at);
        let (next_weekly, next_removal) = if idle >= 8 * WEEK && !protected(tx, project, rule, &settings)? {
            (Some(weekly.unwrap_or(at)), (idle >= 16 * WEEK).then_some(removal.unwrap_or(at)))
        } else { (None, None) };
        if (weekly, removal) == (next_weekly, next_removal) { continue; }
        tx.execute("INSERT INTO supervisor_detector_lifecycle VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT (project, detector) DO UPDATE SET weekly_at = excluded.weekly_at, removal_at = excluded.removal_at",
            (project, rule.id(), since, next_weekly, next_removal)).map_err(sql)?;
        if next_weekly.is_some() { quiet_signals(tx, project, rule)?; }
    }
    Ok(())
}

#[derive(Serialize)]
pub(super) struct Status {
    pub detector: String,
    pub removal: bool,
}

pub(super) fn status(tx: &Transaction, project: &str) -> Result<Vec<Status>, String> {
    rows(tx, "SELECT l.detector, l.removal_at IS NOT NULL FROM supervisor_detector_lifecycle l
        LEFT JOIN supervisor_settings s USING(project)
        WHERE l.project = ?1 AND l.weekly_at IS NOT NULL
        AND coalesce(json_extract(s.pack, '$.detectors.\"' || l.detector || '\".mode'), 'live') = 'live'
        ORDER BY l.detector", [project], |r| {
        Ok(Status { detector: r.get(0)?, removal: r.get(1)? })
    })
}

pub(super) fn shadow_ready(tx: &Transaction, project: &str, detector: &str, since: i64) -> Result<bool, String> {
    tx.query_row("SELECT count(DISTINCT s.fingerprint) >= 3 FROM supervisor_verdicts v
        JOIN supervisor_signals s USING(fingerprint)
        JOIN supervisor_review_reports r ON r.task = v.task AND r.project = s.project
        WHERE r.at >= ?3 AND s.project = ?1 AND s.detector = ?2 AND v.at >= ?3 AND s.last_seen >= ?3 AND s.state = 'shadow'",
        (project, detector, since), |r| r.get(0)).map_err(sql)
}

pub(super) fn mode(settings: &Settings, attachment: Option<&super::attach::Report>,
    detector: &super::Detector, weekly: &BTreeSet<String>, at: i64,
) -> Mode {
    if let Some(mute) = settings.delivery(detector.rule, at) {
        return if mute == Mode::Digest { Mode::Shadow } else { mute };
    }
    let base = attachment.map_or(detector.mode, |report| report.mode(detector, settings));
    if base == Mode::Live && weekly.contains(detector.rule.id()) { Mode::Shadow } else { base }
}

pub(super) fn quieted(tx: &Transaction, project: &str, rule: Rule) -> Result<bool, String> {
    tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_detector_lifecycle
        WHERE project = ?1 AND detector = ?2 AND weekly_at IS NOT NULL)", (project, rule.id()), |r| r.get(0)).map_err(sql)
}
