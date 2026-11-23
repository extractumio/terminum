// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Code-written incident transitions, committed with the originating signal (AC-224–228).

use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::tasks::{ops::{rows, sql}, store::HUMAN};

use super::{Finding, Rule, line_queue::{self, Kind}};

const GROUP_WINDOW: i64 = 15 * 60;
const SNOOZE: i64 = 60 * 60;

pub(super) struct Observation<'a> {
    pub project: &'a str,
    pub rule: Rule,
    pub fingerprint: &'a str,
    pub finding: &'a Finding,
    pub evidence: &'a str,
    pub at: i64,
}

struct Member {
    incident: i64,
    level: u8,
    evidence_hash: Vec<u8>,
    value: f64,
    threshold: f64,
    urgent: bool,
}

pub(super) fn observe(tx: &Transaction, seen: Observation<'_>) -> Result<(), String> {
    let Observation { project, rule, fingerprint, finding, evidence, at } = seen;
    let level = finding.scores.level();
    let urgent = finding.charter_breach || finding.scores.reversibility == 3;
    let mut member = tx.prepare_cached("SELECT incident, level, evidence_hash, value, threshold, urgent
        FROM supervisor_incident_signals WHERE fingerprint = ?1").map_err(sql)?;
    let prior = member.query_row([fingerprint], |r| Ok(Member {
        incident: r.get(0)?, level: r.get(1)?, evidence_hash: r.get(2)?,
        value: r.get(3)?, threshold: r.get(4)?, urgent: r.get(5)?,
    })).optional().map_err(sql)?;
    if level < 3 && prior.is_none() {
        return Ok(());
    }
    let (value, threshold) = (finding.measure.value, finding.measure.threshold);
    let evidence_hash = Sha256::digest(evidence);
    let same_evidence = prior.as_ref().is_some_and(|m| m.evidence_hash.as_slice() == &evidence_hash[..]);
    if same_evidence && prior.as_ref().is_some_and(|m|
        (m.level, m.value, m.threshold, m.urgent) == (level, value, threshold, urgent))
    {
        return Ok(());
    }

    // Only fresh critical evidence extends the grouping window. A level or measurement change
    // against unchanged evidence updates the current incident, even after that window.
    let (incident, opened) = match &prior {
        Some(m) if level < 3 || same_evidence => (m.incident, false),
        _ => find_or_open(tx, project, finding, at)?,
    };
    if opened { super::thread::ensure(tx, project, incident, at)?; }
    super::thread::anchor_live(tx, project, fingerprint, incident, at)?;
    let kind = if level < 3 || same_evidence { "updated" } else { "signal" };
    let event: i64 = tx.query_row("INSERT INTO supervisor_incident_events
        (incident, at, kind, actor, fingerprint, detector, metric, level, value, threshold)
        VALUES (?1, ?2, ?3, 'system', ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
        params![incident, at, kind, fingerprint, rule.id(), finding.metric, level, value, threshold],
        |r| r.get(0)).map_err(sql)?;
    tx.execute("INSERT INTO supervisor_incident_signals VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT (fingerprint) DO UPDATE SET incident = excluded.incident, level = excluded.level,
          evidence_hash = excluded.evidence_hash, value = excluded.value, threshold = excluded.threshold,
          event = excluded.event, urgent = excluded.urgent",
        (fingerprint, incident, level, &evidence_hash[..], value, threshold, event, urgent)).map_err(sql)?;
    line_queue::enqueue(tx, incident, if opened { Kind::Opened } else { Kind::Updated }, false)?;
    if let Some(old) = prior.filter(|m| m.incident != incident)
        && !close_if_clear(tx, old.incident, at, Some(incident), old.urgent)?
    {
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, detector, related_id)
            VALUES (?1, ?2, 'transferred', 'system', ?3, ?4, ?5)",
            (old.incident, at, fingerprint, rule.id(), incident)).map_err(sql)?;
        line_queue::enqueue(tx, old.incident, Kind::Updated, old.urgent)?;
    }
    Ok(())
}

fn find_or_open(tx: &Transaction, project: &str, finding: &Finding, at: i64) -> Result<(i64, bool), String> {
    let related = tx.query_row("SELECT id FROM supervisor_incidents
        WHERE project = ?1 AND subject_kind = ?2 AND subject_id = ?3 AND closed_at IS NULL
          AND last_firing_at BETWEEN ?4 AND ?5 ORDER BY last_firing_at DESC, id DESC LIMIT 1",
        (project, finding.subject_kind, &finding.subject_id, at - GROUP_WINDOW, at), |r| r.get::<_, i64>(0))
        .optional().map_err(sql)?;
    if let Some(id) = related {
        tx.execute("UPDATE supervisor_incidents SET last_firing_at = ?2 WHERE id = ?1", (id, at)).map_err(sql)?;
        Ok((id, false))
    } else {
        tx.query_row("INSERT INTO supervisor_incidents (project, subject_kind, subject_id, opened_at, last_firing_at)
            VALUES (?1, ?2, ?3, ?4, ?4) RETURNING id",
            (project, finding.subject_kind, &finding.subject_id, at), |r| Ok((r.get(0)?, true))).map_err(sql)
    }
}

/// Called after signal resolution in the same transaction. Unavailable readings leave the signal standing.
pub(super) fn clear_resolved(tx: &Transaction, project: &str, rule: Rule, at: i64) -> Result<(), String> {
    let cleared = rows(tx, "SELECT m.fingerprint, m.incident, m.urgent FROM supervisor_signals s
        JOIN supervisor_incident_signals m USING (fingerprint)
        WHERE s.project = ?1 AND s.detector = ?2 AND s.state = 'resolved' ORDER BY m.fingerprint",
        (project, rule.id()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, bool>(2)?)))?;
    for (fingerprint, incident, urgent) in cleared {
        tx.execute("DELETE FROM supervisor_incident_signals WHERE fingerprint = ?1", [&fingerprint]).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, detector)
            VALUES (?1, ?2, 'cleared', 'system', ?3, ?4)", (incident, at, &fingerprint, rule.id())).map_err(sql)?;
        if !close_if_clear(tx, incident, at, None, urgent)? {
            line_queue::enqueue(tx, incident, Kind::Updated, false)?;
        }
    }
    Ok(())
}

fn close_if_clear(tx: &Transaction, id: i64, at: i64, continued: Option<i64>, urgent: bool) -> Result<bool, String> {
    let changed = tx.execute("UPDATE supervisor_incidents SET closed_at = ?2
        WHERE id = ?1 AND closed_at IS NULL
          AND NOT EXISTS (SELECT 1 FROM supervisor_incident_signals WHERE incident = ?1)", (id, at)).map_err(sql)?;
    if changed != 0 {
        let (event, notice) = if continued.is_some() { ("continued", Kind::Updated) } else { ("all_clear", Kind::AllClear) };
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, related_id)
            VALUES (?1, ?2, ?3, 'system', ?4)", (id, at, event, continued)).map_err(sql)?;
        super::thread::clear(tx, id, at)?;
        line_queue::stop_repeats(tx, id)?;
        line_queue::enqueue(tx, id, notice, urgent)?;
    }
    Ok(changed != 0)
}

pub(super) fn revision(tx: &Transaction) -> Result<i64, String> {
    tx.query_row("SELECT coalesce(max(id), 0) FROM supervisor_incident_events", [], |r| r.get(0)).map_err(sql)
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Action { Acknowledge, Snooze }

pub(super) fn act(tx: &Transaction, id: i64, action: Action, at: i64) -> Result<bool, String> {
    let (changed, kind) = match action {
        Action::Acknowledge => (tx.execute("UPDATE supervisor_incidents SET acknowledged_at = ?2, snoozed_until = NULL
            WHERE id = ?1 AND acknowledged_at IS NULL", (id, at)), "acknowledged"),
        Action::Snooze => (tx.execute("UPDATE supervisor_incidents SET snoozed_until = ?3
            WHERE id = ?1 AND closed_at IS NULL AND acknowledged_at IS NULL AND coalesce(snoozed_until, 0) <= ?2",
            (id, at, at + SNOOZE)), "snoozed"),
    };
    let changed = changed.map_err(sql)? != 0;
    if changed {
        // Acknowledge stops repeats in the claim (AC-228); the row's last send keeps the updates' cool-down.
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor)
            VALUES (?1, ?2, ?3, ?4)", (id, at, kind, HUMAN)).map_err(sql)?;
    }
    Ok(changed)
}
