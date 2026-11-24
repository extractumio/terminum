// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Typed notices and their durable delivery schedule (AC-227–228).

use rusqlite::{OptionalExtension, Transaction};
use crate::tasks::ops::sql;
use super::line::Settings;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind { Opened, Updated, AllClear, Answer, Repeat }

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Opened => "opened", Self::Updated => "updated", Self::AllClear => "all_clear",
            Self::Answer => "answer", Self::Repeat => "repeat",
        }
    }

    fn read(s: &str) -> Self {
        match s {
            "opened" => Self::Opened, "updated" => Self::Updated, "all_clear" => Self::AllClear,
            "answer" => Self::Answer, "repeat" => Self::Repeat,
            _ => unreachable!("checked delivery kind"),
        }
    }
}

#[derive(Clone)]
pub(super) struct Notice {
    pub project: String,
    pub incident: i64,
    pub kind: Kind,
    pub level: u8,
    pub signals: i64,
    pub channels: usize,
    pub(super) budget: Option<&'static str>,
    /// Each hold in force, and whether it is an interim verification hold (AC-233).
    pub(super) holds: Vec<(super::holds::Kind, bool)>,
}

impl Notice {
    pub fn render(&self, link_base: Option<&str>) -> String {
        let Self { incident, level, signals, .. } = self;
        let mut text = match self.kind {
            Kind::Opened | Kind::Repeat => format!("Critical incident {incident} · Level {level}. Active signals: {signals}. Acknowledge in Terminum.\n"),
            Kind::Updated => format!("Update on incident {incident} · Level {level}. Active signals: {signals}.\n"),
            Kind::AllClear => format!("All-clear on incident {incident}.\n"),
            Kind::Answer => format!("An answer on incident {incident}.\n"),
        };
        if !self.holds.is_empty() && matches!(self.kind, Kind::Opened | Kind::Updated | Kind::Repeat) {
            use std::fmt::Write;
            for (hold, interim) in &self.holds {
                let _ = match interim {
                    true => writeln!(text, "{} while its verification runs, for up to {} minutes.", hold.label(), super::rounds::TIME_LIMIT / 60),
                    false => writeln!(text, "{}.", hold.label()),
                };
            }
            let end = if self.holds.iter().all(|(_, interim)| *interim) { super::holds::INTERIM_END } else { "Without an answer the hold stays." };
            let _ = writeln!(text, "Keep or release in Terminum. {end}");
        }
        // Only generated project IDs are eligible for the outbound deep link.
        if let Some(base) = link_base
            && (self.project == "default"
                || (self.project.len() == 12 && self.project.bytes().all(|b| b.is_ascii_hexdigit()))
                || (!self.project.is_empty() && self.project.len() <= 20 && self.project.bytes().all(|b| b.is_ascii_digit())))
        {
            use std::fmt::Write;
            let _ = writeln!(text, "{base}/supervisor/?project={}&incident={incident}#incident-{incident}", self.project);
        }
        match self.budget { Some(budget) => format!("{budget}\n{text}"), None => text }
    }
}

const ALERTS: &str = "(SELECT json_group_array(a.id) FROM supervisor_incident_signals m
    JOIN supervisor_actions a ON a.kind = 'alert' AND a.project = i.project
      AND a.target = CAST(i.id AS TEXT) AND a.fingerprint = m.fingerprint
      AND a.id = (SELECT max(v.id) FROM supervisor_actions v WHERE v.project = a.project
        AND v.kind = a.kind AND v.target = a.target AND v.fingerprint = a.fingerprint)
    WHERE m.incident = i.id)";

const ALERT_MEMBERS: &str = "LEFT JOIN json_each(ready.alerts) list ON true
    LEFT JOIN supervisor_actions a ON a.id = list.value AND a.project = ready.project AND a.kind = 'alert'
    LEFT JOIN supervisor_incident_signals m ON m.fingerprint = a.fingerprint AND m.incident = ready.incident
    LEFT JOIN supervisor_breakers b ON b.project = a.project AND b.detector = a.detector
      AND b.response = a.response AND b.closed_at IS NULL";

/// The current numbers of the incidents `filter` selects.
/// No configured channel, or a project whose Supervisor is off, queues nothing.
fn snapshot(filter: &str) -> String {
    format!("SELECT i.id AS incident, coalesce(max(m.level), (
            SELECT max(level) FROM supervisor_holds WHERE incident = i.id AND shadow = 0 AND released_at IS NULL), 0) AS level,
            count(m.fingerprint) AS signals, coalesce(max(m.urgent), 0) AS urgent, {ALERTS} AS alerts
        FROM supervisor_incidents i LEFT JOIN supervisor_incident_signals m ON m.incident = i.id
        WHERE {filter} AND i.subject_kind <> 'critical_line'
          AND EXISTS (SELECT 1 FROM supervisor_line WHERE json_array_length(settings, '$.channels') > 0)
          AND EXISTS (SELECT 1 FROM project_supervisor s WHERE s.project = i.project AND s.enabled)
        GROUP BY i.id")
}

/// Snapshots numeric notice fields in the same transaction as the timeline change.
/// The incident thread (AC-244) can enqueue an answer using only the incident ID.
/// An incident keeps at most one waiting incident or update notice (AC-228): a change
/// refreshes its numbers and urgency, and an all-clear replaces it, so a backlog that
/// waited through quiet hours carries only the incident's latest state.
pub(super) fn enqueue(tx: &Transaction, incident: i64, kind: Kind, ended_urgent: bool) -> Result<(), String> {
    debug_assert_ne!(kind, Kind::Repeat);
    let fresh = snapshot("i.id = ?1");
    let waiting = "SELECT id FROM supervisor_line_pending WHERE incident = ?1 AND kind IN ('opened', 'updated')";
    match kind {
        Kind::Opened | Kind::Updated => {
            let row = tx.query_row(&format!("{waiting} ORDER BY id LIMIT 1"), [incident], |r| r.get::<_, i64>(0))
                .optional().map_err(sql)?;
            if let Some(row) = row {
                return tx.execute(&format!("UPDATE supervisor_line_pending AS q
                    SET level = f.level, signals = f.signals, urgent = (f.urgent OR ?2), alerts = f.alerts
                    FROM ({fresh}) f WHERE q.id = ?3"), (incident, ended_urgent, row)).map(drop).map_err(sql);
            }
        }
        Kind::AllClear => {
            tx.execute(&format!("DELETE FROM supervisor_line_pending WHERE id IN ({waiting})"), [incident]).map_err(sql)?;
        }
        Kind::Answer | Kind::Repeat => {}
    }
    tx.execute(&format!("INSERT INTO supervisor_line_pending (incident, kind, level, signals, urgent, alerts)
        SELECT incident, ?2, level, signals, (urgent OR ?3), alerts FROM ({fresh})"), (incident, kind.name(), ended_urgent))
        .map(drop).map_err(sql)
}

/// Queues the current unacknowledged incidents, of one project or all: when the first channel
/// is saved, and when a project's Supervisor is turned back on.
pub(super) fn seed(tx: &Transaction, project: Option<&str>) -> Result<(), String> {
    let open = snapshot("(?1 IS NULL OR i.project = ?1) AND i.closed_at IS NULL AND i.acknowledged_at IS NULL");
    tx.execute(&format!("INSERT INTO supervisor_line_pending (incident, kind, level, signals, urgent, alerts)
        SELECT incident, 'opened', level, signals, urgent, alerts FROM ({open})"), [project]).map(drop).map_err(sql)
}

/// Drops the waiting notices and repeat steps, of one project or all.
pub(super) fn clear(tx: &Transaction, project: Option<&str>) -> Result<(), String> {
    for table in ["supervisor_line_pending", "supervisor_line_repeats"] {
        tx.execute(&format!("DELETE FROM {table} WHERE ?1 IS NULL OR incident IN (
            SELECT id FROM supervisor_incidents WHERE project = ?1)"), [project]).map_err(sql)?;
    }
    Ok(())
}

/// A project's Supervisor turned off or on (AC-228): its queued notices and repeat steps
/// are stale and go; turned on, its open, unacknowledged incidents reach the owner once.
pub(super) fn restart(tx: &Transaction, project: &str, on: bool) -> Result<(), String> {
    clear(tx, Some(project))?;
    if on { seed(tx, Some(project))?; }
    Ok(())
}

pub(super) fn stop_repeats(tx: &Transaction, id: i64) -> Result<(), String> {
    tx.execute("DELETE FROM supervisor_line_repeats WHERE incident = ?1", [id]).map(drop).map_err(sql)
}

pub(super) enum Claim {
    Empty,
    Discarded,
    Notice(Notice),
}

/// Claim once, before starting an external program. A crash must not replay a
/// one-shot notification or send several missed repeat steps at once.
pub(super) fn claim(tx: &Transaction, settings: &Settings, at: i64, quiet: bool) -> Result<Claim, String> {
    let first = settings.delay(0);
    // A waiting page the owner no longer needs goes (AC-228): its incident was acknowledged, or
    // it closed because its signals moved on to a new incident, whose own page covers it.
    tx.execute("DELETE FROM supervisor_line_pending AS q WHERE kind = 'opened' AND EXISTS (
        SELECT 1 FROM supervisor_incidents i WHERE i.id = q.incident
          AND (i.acknowledged_at IS NOT NULL OR i.closed_at IS NOT NULL))", []).map_err(sql)?;
    let selected = tx.query_row(&format!("WITH ready AS (
        SELECT q.id AS pending, i.id AS incident, i.project, q.kind,
               CASE WHEN q.kind IN ('all_clear', 'answer') THEN 0 ELSE q.level END AS level, q.signals,
               0 AS escalated, CASE WHEN q.kind NOT IN ('all_clear', 'answer') THEN q.alerts END AS alerts
        FROM supervisor_line_pending q JOIN supervisor_incidents i ON i.id = q.incident
        JOIN project_supervisor s ON s.project = i.project AND s.enabled
        LEFT JOIN supervisor_line_repeats r ON r.incident = i.id
        WHERE (NOT ?2 OR (q.urgent AND (q.kind IN ('all_clear','answer') OR EXISTS (
            SELECT 1 FROM supervisor_incident_signals m JOIN supervisor_signals s USING (fingerprint)
            WHERE m.incident = i.id AND m.urgent AND NOT EXISTS (SELECT 1 FROM supervisor_breakers b
              WHERE b.project = i.project AND b.detector = s.detector AND b.response = CASE m.level WHEN 3 THEN 'R5' ELSE 'R4' END
                AND b.closed_at IS NULL)))))
          AND q.id = (SELECT min(id) FROM supervisor_line_pending WHERE incident = i.id)
          AND (q.kind <> 'updated' OR r.last_at IS NULL OR r.last_at + ?3 <= ?1 OR i.closed_at IS NOT NULL)
        UNION ALL
        SELECT NULL, i.id, i.project, 'repeat', coalesce(max(m.level), (
            SELECT max(level) FROM supervisor_holds WHERE incident = i.id AND shadow = 0 AND released_at IS NULL), 0),
            count(m.fingerprint), r.escalated, {ALERTS}
        FROM supervisor_line_repeats r JOIN supervisor_incidents i ON i.id = r.incident
        JOIN project_supervisor s ON s.project = i.project AND s.enabled
        LEFT JOIN supervisor_incident_signals m ON m.incident = i.id
        WHERE r.due_at <= ?1 AND i.closed_at IS NULL AND i.acknowledged_at IS NULL
          AND coalesce(i.snoozed_until, 0) <= ?1 AND r.last_at + ?3 <= ?1
        GROUP BY i.id HAVING NOT ?2 OR max(m.urgent AND NOT EXISTS (
            SELECT 1 FROM supervisor_signals s JOIN supervisor_breakers b ON b.project = s.project AND b.detector = s.detector
            WHERE s.fingerprint = m.fingerprint AND b.response = CASE m.level WHEN 3 THEN 'R5' ELSE 'R4' END AND b.closed_at IS NULL))
    ), ranked AS (
        SELECT ready.*,
               CASE WHEN count(a.id) = 0 THEN ready.level
                    ELSE coalesce(max(m.level) FILTER (WHERE b.id IS NULL), 0) END AS priority
        FROM ready {ALERT_MEMBERS}
        GROUP BY ready.pending, ready.incident
    ) SELECT pending, incident, project, kind, priority, signals, escalated, alerts
      , (SELECT s.metric FROM supervisor_signals s WHERE s.project = ranked.project AND s.detector = 'self.budget'
            AND s.state = 'raised' AND s.metric IN ('judgment_tokens','fix_dollars')
          ORDER BY s.metric = 'judgment_tokens' DESC LIMIT 1)
      FROM ranked ORDER BY priority DESC, incident, pending LIMIT 1"), (at, quiet, first), |r| {
        let kind = Kind::read(&r.get::<_, String>(3)?);
        let escalated: bool = r.get(6)?;
        let notice = Notice { incident: r.get(1)?, project: r.get(2)?, kind,
            level: r.get(4)?, signals: r.get(5)?, channels: if kind == Kind::Repeat && escalated { 2 } else { 1 },
            budget: match r.get::<_, Option<String>>(8)?.as_deref() {
                Some("judgment_tokens") => Some(super::governor::JUDGMENT_NOTICE),
                Some("fix_dollars") => Some(super::governor::FIX_NOTICE),
                _ => None,
            }, holds: Vec::new() };
        Ok((r.get::<_, Option<i64>>(0)?, notice, escalated, r.get::<_, Option<String>>(7)?))
    }).optional().map_err(sql)?;
    let Some((pending, mut notice, escalated, alerts)) = selected else { return Ok(Claim::Empty) };
    let id = notice.incident;
    if let Some(row) = pending {
        tx.execute("DELETE FROM supervisor_line_pending WHERE id = ?1", [row]).map_err(sql)?;
    }
    if !alert_allowed(tx, &mut notice, alerts.as_deref(), at)? {
        stop_repeats(tx, id)?;
        return Ok(Claim::Discarded);
    }
    notice.holds = crate::tasks::ops::rows(tx, "SELECT h.kind, h.until IS NOT NULL FROM supervisor_holds h
        WHERE h.incident = ?1 AND h.shadow = 0 AND h.released_at IS NULL
          AND NOT EXISTS (SELECT 1 FROM supervisor_breakers b WHERE b.project = h.project
            AND b.detector = h.detector AND b.response = h.response AND b.closed_at IS NULL) ORDER BY h.id", [id],
        |r| Ok((super::holds::Kind::read(&r.get::<_, String>(0)?), r.get(1)?)))?;
    match notice.kind {
        // The row's last_at is every open incident's update cool-down, acknowledged or not;
        // the repeat branch above skips acknowledged incidents.
        Kind::Opened | Kind::Updated => {
            tx.execute("INSERT INTO supervisor_line_repeats (incident, escalated, last_at, due_at)
                SELECT id, 0, ?2, ?3 FROM supervisor_incidents WHERE id = ?1 AND closed_at IS NULL
                ON CONFLICT (incident) DO UPDATE SET last_at = excluded.last_at", (id, at, at + first)).map_err(sql)?;
        }
        Kind::Repeat => {
            let delay = if escalated { settings.delay(2) } else { settings.delay(1) - first };
            tx.execute("UPDATE supervisor_line_repeats SET escalated = 1, last_at = ?2, due_at = ?3
                WHERE incident = ?1", (id, at, at + delay)).map_err(sql)?;
        }
        Kind::AllClear | Kind::Answer => {}
    }
    Ok(Claim::Notice(notice))
}

// A grouped notice can still go out for its healthy rules. Check at claim so
// already queued alerts, seeded alerts and the repeat ladder use the same gate.
fn alert_allowed(tx: &Transaction, notice: &mut Notice, alerts: Option<&str>, at: i64) -> Result<bool, String> {
    let Some(alerts) = alerts else { return Ok(true) };
    let members = crate::tasks::ops::rows(tx, &format!("WITH ready(project, incident, alerts) AS (VALUES (?1, ?2, ?3))
        SELECT a.fingerprint, a.response, b.id IS NOT NULL, m.level
        FROM ready {ALERT_MEMBERS} WHERE a.id IS NOT NULL ORDER BY a.id"), (&notice.project, notice.incident, alerts),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?, r.get::<_, Option<u8>>(3)?)))?;
    if members.is_empty() { return Ok(true); }
    notice.signals = 0;
    for (key, response, blocked, level) in members {
        if blocked {
            let signal = super::signals::get(tx, &notice.project, &key)?;
            super::breakers::divert_rule(tx, &signal, &response, "alert", "", "The incident would send another alert.", at)?;
        } else if level.is_some() {
            notice.signals += 1;
        }
    }
    Ok(notice.signals > 0)
}
