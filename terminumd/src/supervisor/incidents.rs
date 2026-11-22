// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Code-written incident transitions, committed with the originating signal.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::tasks::{ops::{rows, sql}, store::HUMAN};

use super::{Finding, Rule};

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

pub(super) fn observe(tx: &Transaction, seen: Observation<'_>) -> Result<(), String> {
    let Observation { project, rule, fingerprint, finding, evidence, at } = seen;
    let level = finding.scores.level();
    let mut member = tx.prepare_cached("SELECT incident, level, evidence_hash, value, threshold
        FROM supervisor_incident_signals WHERE fingerprint = ?1").map_err(sql)?;
    let prior = member.query_row([fingerprint], |r| Ok((
        r.get::<_, i64>(0)?, r.get::<_, u8>(1)?, r.get::<_, Vec<u8>>(2)?, r.get::<_, f64>(3)?, r.get::<_, f64>(4)?,
    ))).optional().map_err(sql)?;
    if level < 3 && prior.is_none() {
        return Ok(());
    }
    let (value, threshold) = (finding.measure.value, finding.measure.threshold);
    let evidence_hash = Sha256::digest(evidence);
    let same_evidence = prior.as_ref().is_some_and(|(_, _, hash, _, _)| hash.as_slice() == &evidence_hash[..]);
    if same_evidence && prior.as_ref().is_some_and(|(_, old_level, _, old_value, old_threshold)|
        (*old_level, *old_value, *old_threshold) == (level, value, threshold))
    {
        return Ok(());
    }

    // Only fresh critical evidence extends the grouping window. A level or measurement change
    // against unchanged evidence updates the current incident, even after that window.
    let incident = match &prior {
        Some((id, ..)) if level < 3 || same_evidence => *id,
        _ => find_or_open(tx, project, finding, at)?,
    };
    let kind = if level < 3 || same_evidence { "updated" } else { "signal" };
    let event: i64 = tx.query_row("INSERT INTO supervisor_incident_events
        (incident, at, kind, actor, fingerprint, detector, metric, level, value, threshold)
        VALUES (?1, ?2, ?3, 'system', ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
        params![incident, at, kind, fingerprint, rule.id(), finding.metric, level, value, threshold],
        |r| r.get(0)).map_err(sql)?;
    tx.execute("INSERT INTO supervisor_incident_signals VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT (fingerprint) DO UPDATE SET incident = excluded.incident, level = excluded.level,
          evidence_hash = excluded.evidence_hash, value = excluded.value, threshold = excluded.threshold, event = excluded.event",
        (fingerprint, incident, level, &evidence_hash[..], value, threshold, event)).map_err(sql)?;
    if let Some((old, ..)) = prior.filter(|(old, ..)| *old != incident)
        && !close_if_clear(tx, old, at, Some(incident))?
    {
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, detector, related_id)
            VALUES (?1, ?2, 'transferred', 'system', ?3, ?4, ?5)",
            (old, at, fingerprint, rule.id(), incident)).map_err(sql)?;
    }
    Ok(())
}

fn find_or_open(tx: &Transaction, project: &str, finding: &Finding, at: i64) -> Result<i64, String> {
    let related = tx.query_row("SELECT id FROM supervisor_incidents
        WHERE project = ?1 AND subject_kind = ?2 AND subject_id = ?3 AND closed_at IS NULL
          AND last_firing_at BETWEEN ?4 AND ?5 ORDER BY last_firing_at DESC, id DESC LIMIT 1",
        (project, finding.subject_kind, &finding.subject_id, at - GROUP_WINDOW, at), |r| r.get::<_, i64>(0))
        .optional().map_err(sql)?;
    if let Some(id) = related {
        tx.execute("UPDATE supervisor_incidents SET last_firing_at = ?2 WHERE id = ?1", (id, at)).map_err(sql)?;
        Ok(id)
    } else {
        tx.query_row("INSERT INTO supervisor_incidents (project, subject_kind, subject_id, opened_at, last_firing_at)
            VALUES (?1, ?2, ?3, ?4, ?4) RETURNING id",
            (project, finding.subject_kind, &finding.subject_id, at), |r| r.get(0)).map_err(sql)
    }
}

/// Called after signal resolution in the same transaction. Unavailable readings leave the signal standing.
pub(super) fn clear_resolved(tx: &Transaction, project: &str, rule: Rule, at: i64) -> Result<(), String> {
    let cleared = rows(tx, "SELECT m.fingerprint, m.incident FROM supervisor_signals s
        JOIN supervisor_incident_signals m USING (fingerprint)
        WHERE s.project = ?1 AND s.detector = ?2 AND s.state = 'resolved' ORDER BY m.fingerprint",
        (project, rule.id()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    for (fingerprint, incident) in cleared {
        tx.execute("DELETE FROM supervisor_incident_signals WHERE fingerprint = ?1", [&fingerprint]).map_err(sql)?;
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, detector)
            VALUES (?1, ?2, 'cleared', 'system', ?3, ?4)", (incident, at, &fingerprint, rule.id())).map_err(sql)?;
        close_if_clear(tx, incident, at, None)?;
    }
    Ok(())
}

fn close_if_clear(tx: &Transaction, id: i64, at: i64, continued: Option<i64>) -> Result<bool, String> {
    let changed = tx.execute("UPDATE supervisor_incidents SET closed_at = ?2
        WHERE id = ?1 AND closed_at IS NULL
          AND NOT EXISTS (SELECT 1 FROM supervisor_incident_signals WHERE incident = ?1)", (id, at)).map_err(sql)?;
    if changed != 0 {
        let kind = if continued.is_some() { "continued" } else { "all_clear" };
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, related_id)
            VALUES (?1, ?2, ?3, 'system', ?4)", (id, at, kind, continued)).map_err(sql)?;
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
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor)
            VALUES (?1, ?2, ?3, ?4)", (id, at, kind, HUMAN)).map_err(sql)?;
    }
    Ok(changed)
}
