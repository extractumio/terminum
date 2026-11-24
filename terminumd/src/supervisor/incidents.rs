// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Code-written incident transitions, committed with the originating signal (AC-224–228).

use rusqlite::{OptionalExtension, Transaction, params};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::tasks::{ops::{rows, sql}, store::HUMAN};

use super::{Finding, Rule, line_queue::{self, Kind}};

pub(super) const GROUP_WINDOW: i64 = 15 * 60;
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
    // A round's standing escalation raises the level (AC-213); urgency stays the detector's own scores.
    let level = super::escalations::standing(tx, fingerprint, evidence)?
        .map_or(finding.scores, |score| score.raise(finding.scores)).level();
    let urgent = finding.charter_breach() || finding.scores.reversibility == 3;
    let mut member = tx.prepare_cached("SELECT incident, level, evidence_hash, value, threshold, urgent
        FROM supervisor_incident_signals WHERE fingerprint = ?1").map_err(sql)?;
    let prior = member.query_row([fingerprint], |r| Ok(Member {
        incident: r.get(0)?, level: r.get(1)?, evidence_hash: r.get(2)?,
        value: r.get(3)?, threshold: r.get(4)?, urgent: r.get(5)?,
    })).optional().map_err(sql)?;
    if level < 3 && prior.is_none() {
        return Ok(());
    }
    if level >= 3 && super::breakers::open_id(tx, project, rule.id(), finding.scores.response(false))?.is_some() {
        let signal = super::signals::get(tx, project, fingerprint)?;
        if super::breakers::divert(tx, &signal, "alert", "", "The detector would raise a critical alert.", at)?.is_some() {
            return Ok(());
        }
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
        _ => find_or_open(tx, project, (finding.subject_kind, &finding.subject_id), at)?,
    };
    if opened {
        super::thread::ensure(tx, project, incident, at)?;
        // its critical report in the Inbox, alerting through the incident alone (terminum-reports.md AC-271, AC-272)
        let why = format!("{} on {}: {} against a threshold of {}", rule.id(), finding.metric, value, threshold);
        crate::reports::sources::incident(tx, project, incident, &format!("{} {}", finding.subject_kind, finding.subject_id), &why, at)?;
    }
    let kind = if level < 3 || same_evidence { "updated" } else { "signal" };
    record_signal(tx, project, fingerprint, (rule.id(), finding.metric, kind),
        Member { incident, level, value, threshold, urgent, evidence_hash: evidence_hash.to_vec() }, at)?;
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

fn record_signal(tx: &Transaction, project: &str, fingerprint: &str, words: (&str, &str, &str),
    member: Member, at: i64) -> Result<(), String>
{
    super::thread::anchor_live(tx, project, fingerprint, member.incident, at)?;
    let event: i64 = tx.query_row("INSERT INTO supervisor_incident_events
        (incident, at, kind, actor, fingerprint, detector, metric, level, value, threshold)
        VALUES (?1, ?2, ?3, 'system', ?4, ?5, ?6, ?7, ?8, ?9) RETURNING id",
        params![member.incident, at, words.2, fingerprint, words.0, words.1, member.level, member.value, member.threshold],
        |r| r.get(0)).map_err(sql)?;
    let signal = super::signals::get(tx, project, fingerprint)?;
    super::outcomes::record(tx, &signal, super::outcomes::Kind::Alert, &event.to_string(), &member.incident.to_string(), "", at)?;
    tx.execute("INSERT INTO supervisor_incident_signals VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT (fingerprint) DO UPDATE SET incident = excluded.incident, level = excluded.level,
          evidence_hash = excluded.evidence_hash, value = excluded.value, threshold = excluded.threshold,
          event = excluded.event, urgent = excluded.urgent",
        (fingerprint, member.incident, member.level, &member.evidence_hash, member.value,
            member.threshold, event, member.urgent)).map(drop).map_err(sql)
}

fn find_or_open(tx: &Transaction, project: &str, subject: (&str, &str), at: i64) -> Result<(i64, bool), String> {
    let related = tx.query_row("SELECT id FROM supervisor_incidents
        WHERE project = ?1 AND subject_kind = ?2 AND subject_id = ?3 AND closed_at IS NULL
          AND last_firing_at BETWEEN ?4 AND ?5 ORDER BY last_firing_at DESC, id DESC LIMIT 1",
        (project, subject.0, subject.1, at - GROUP_WINDOW, at), |r| r.get::<_, i64>(0))
        .optional().map_err(sql)?;
    if let Some(id) = related {
        tx.execute("UPDATE supervisor_incidents SET last_firing_at = ?2 WHERE id = ?1", (id, at)).map_err(sql)?;
        Ok((id, false))
    } else {
        tx.query_row("INSERT INTO supervisor_incidents (project, subject_kind, subject_id, opened_at, last_firing_at)
            VALUES (?1, ?2, ?3, ?4, ?4) RETURNING id",
            (project, subject.0, subject.1, at), |r| Ok((r.get(0)?, true))).map_err(sql)
    }
}

/// The open incident a signal belongs to.
fn open_incident_of(tx: &Transaction, fingerprint: &str) -> Result<Option<i64>, String> {
    tx.query_row("SELECT m.incident FROM supervisor_incident_signals m
        JOIN supervisor_incidents i ON i.id = m.incident WHERE m.fingerprint = ?1 AND i.closed_at IS NULL",
        [fingerprint], |r| r.get(0)).optional().map_err(sql)
}

/// Join a stored signal to `incident`, as code's own event `kind`, with its stored evidence.
fn join(tx: &Transaction, signal: &super::Signal, incident: i64, kind: &str, at: i64) -> Result<(), String> {
    let evidence: String = tx.query_row("SELECT evidence FROM supervisor_signals WHERE fingerprint = ?1",
        [&signal.fingerprint], |r| r.get(0)).map_err(sql)?;
    let member = Member { incident, level: signal.level, value: signal.measure.value,
        threshold: signal.measure.threshold, urgent: signal.charter_breach || signal.scores.reversibility == 3,
        evidence_hash: Sha256::digest(evidence).to_vec() };
    record_signal(tx, &signal.project, &signal.fingerprint, (&signal.detector, &signal.metric, kind), member, at)
}

/// Open, or find within the grouping window, the incident of a signal's subject.
fn open_for(tx: &Transaction, signal: &super::Signal, at: i64) -> Result<(i64, bool), String> {
    let (id, opened) = find_or_open(tx, &signal.project, (&signal.subject_kind, &signal.subject_id), at)?;
    if opened { super::thread::ensure(tx, &signal.project, id, at)?; }
    Ok((id, opened))
}

/// A project-wide hold opens an incident even when its signal is at level 2.
pub(super) fn attach_hold(tx: &Transaction, signal: &super::Signal, hold: i64, project_wide: bool,
    actor: &str, at: i64) -> Result<(), String>
{
    let (id, opened) = match open_incident_of(tx, &signal.fingerprint)? {
        Some(id) => (id, false),
        None if !project_wide => return Ok(()),
        None => {
            let (id, opened) = open_for(tx, signal, at)?;
            join(tx, signal, id, "signal", at)?;
            (id, opened)
        }
    };
    tx.execute("UPDATE supervisor_holds SET incident = ?2 WHERE id = ?1", (hold, id)).map_err(sql)?;
    append_hold_event(tx, id, hold, "hold_applied", actor, at)?;
    line_queue::enqueue(tx, id, if opened { Kind::Opened } else { Kind::Updated }, false)
}

/// A checked round's critical escalation (AC-213) opens or raises its signal's incident at once.
pub(super) fn escalate(tx: &Transaction, signal: &super::Signal, at: i64) -> Result<(), String> {
    if super::breakers::divert(tx, signal, "alert", "", "A round escalated the signal to a critical alert.", at)?.is_some() {
        return Ok(());
    }
    let (incident, opened) = match open_incident_of(tx, &signal.fingerprint)? {
        Some(id) => (id, false),
        None => open_for(tx, signal, at)?,
    };
    join(tx, signal, incident, "escalated", at)?;
    line_queue::enqueue(tx, incident, if opened { Kind::Opened } else { Kind::Updated }, false)
}

fn append_hold_event(tx: &Transaction, incident: i64, hold: i64, kind: &str, actor: &str, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, related_id)
        VALUES (?1, ?2, ?3, ?4, ?5)", (incident, at, kind, actor, hold)).map(drop).map_err(sql)
}

pub(super) fn hold_event(tx: &Transaction, incident: i64, hold: i64, kind: &str, actor: &str, at: i64) -> Result<(), String> {
    append_hold_event(tx, incident, hold, kind, actor, at)?;
    if kind == "hold_released" && !close_if_clear(tx, incident, at, None, false)? {
        line_queue::enqueue(tx, incident, Kind::Updated, false)?;
    }
    Ok(())
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
          AND NOT EXISTS (SELECT 1 FROM supervisor_incident_signals WHERE incident = ?1)
          AND NOT EXISTS (SELECT 1 FROM supervisor_holds WHERE incident = ?1 AND shadow = 0 AND released_at IS NULL)", (id, at)).map_err(sql)?;
    if changed != 0 {
        let (event, notice) = if continued.is_some() { ("continued", Kind::Updated) } else { ("all_clear", Kind::AllClear) };
        tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, related_id)
            VALUES (?1, ?2, ?3, 'system', ?4)", (id, at, event, continued)).map_err(sql)?;
        super::thread::clear(tx, id, at)?;
        if continued.is_none()
            && let Some(project) = tx.query_row("SELECT project FROM supervisor_incidents WHERE id = ?1 AND EXISTS (
                SELECT 1 FROM supervisor_incident_events e WHERE e.incident = ?1 AND e.level = 3)",
                [id], |r| r.get::<_, String>(0)).optional().map_err(sql)?
        {
            super::reflection::enqueue(tx, &project, ("incident", &id.to_string(), None),
                "incident_postmortem", &serde_json::json!({"incident":id}), at)?;
        }
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

/// A grouped incident may belong to several rules. The owner identifies one retained alert.
pub(super) fn wrong_alert(tx: &Transaction, incident: i64, id: i64, at: i64) -> Result<bool, String> {
    let row = tx.query_row("SELECT i.project, a.source FROM supervisor_incidents i
        JOIN supervisor_actions a ON a.project = i.project AND a.target = CAST(i.id AS TEXT)
        WHERE i.id = ?1 AND a.id = ?2 AND a.kind = 'alert'", (incident, id),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional().map_err(sql)?;
    let (project, source) = row.ok_or("Choose a recorded alert on this incident.")?;
    let a = super::outcomes::find(tx, &project, super::outcomes::Kind::Alert, &source)?
        .ok_or("Choose a recorded alert on this incident.")?;
    if !super::outcomes::judge(tx, &project, &a, 0, at, (super::outcomes::Verdict::Wrong, "The owner marked the alert wrong."))? {
        return Ok(false);
    }
    tx.execute("INSERT INTO supervisor_incident_events (incident, at, kind, actor, fingerprint, detector, related_id)
        VALUES (?1, ?2, 'alert_wrong', ?3, ?4, ?5, ?6)", (incident, at, HUMAN, &a.fingerprint, &a.detector, a.id)).map_err(sql)?;
    Ok(true)
}
