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

pub(super) struct Notice {
    pub project: String,
    pub incident: i64,
    pub kind: Kind,
    pub level: u8,
    pub signals: i64,
    pub channels: usize,
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
        // Only generated project IDs are eligible for the outbound deep link.
        if let Some(base) = link_base
            && (self.project == "default"
                || (self.project.len() == 12 && self.project.bytes().all(|b| b.is_ascii_hexdigit()))
                || (!self.project.is_empty() && self.project.len() <= 20 && self.project.bytes().all(|b| b.is_ascii_digit())))
        {
            use std::fmt::Write;
            let _ = writeln!(text, "{base}/supervisor/?project={}&incident={incident}#incident-{incident}", self.project);
        }
        text
    }
}

/// Snapshots numeric notice fields in the same transaction as the timeline change.
/// The incident thread (AC-244) can enqueue an answer using only the incident ID.
pub(super) fn enqueue(tx: &Transaction, incident: i64, kind: Kind, ended_urgent: bool) -> Result<(), String> {
    debug_assert_ne!(kind, Kind::Repeat);
    if kind == Kind::Updated {
        tx.execute("DELETE FROM supervisor_line_pending WHERE kind = 'updated' AND id = (
            SELECT max(id) FROM supervisor_line_pending WHERE incident = ?1)", [incident]).map_err(sql)?;
    }
    tx.execute("INSERT INTO supervisor_line_pending (incident, kind, level, signals, urgent)
        SELECT i.id, ?2, coalesce(max(m.level), 0), count(m.fingerprint), (?3 OR coalesce(max(m.urgent), 0))
        FROM supervisor_incidents i LEFT JOIN supervisor_incident_signals m ON m.incident = i.id
        WHERE i.id = ?1 AND i.subject_kind <> 'critical_line'
          AND EXISTS (SELECT 1 FROM supervisor_line WHERE json_array_length(settings, '$.channels') > 0)
        GROUP BY i.id", (incident, kind.name(), ended_urgent)).map(drop).map_err(sql)
}

/// When the first channel is saved, start with current unacknowledged incidents.
pub(super) fn seed(tx: &Transaction) -> Result<(), String> {
    tx.execute("INSERT INTO supervisor_line_pending (incident, kind, level, signals, urgent)
        SELECT i.id, 'opened', max(m.level), count(*), max(m.urgent)
        FROM supervisor_incidents i JOIN supervisor_incident_signals m ON m.incident = i.id
        WHERE i.closed_at IS NULL AND i.acknowledged_at IS NULL AND i.subject_kind <> 'critical_line'
        GROUP BY i.id", []).map(drop).map_err(sql)
}

pub(super) fn stop_repeats(tx: &Transaction, id: i64) -> Result<(), String> {
    tx.execute("DELETE FROM supervisor_line_repeats WHERE incident = ?1", [id]).map(drop).map_err(sql)
}

/// Claim once, before starting an external program. A crash must not replay a
/// one-shot notification or send several missed repeat steps at once.
pub(super) fn claim(tx: &Transaction, settings: &Settings, at: i64, quiet: bool) -> Result<Option<Notice>, String> {
    let first = settings.delay(0);
    let selected = tx.query_row("WITH ready AS (
        SELECT q.id AS pending, i.id AS incident, i.project, q.kind, CASE WHEN q.kind IN ('all_clear', 'answer') THEN 0 ELSE q.level END AS level, q.signals,
               0 AS escalated
        FROM supervisor_line_pending q JOIN supervisor_incidents i ON i.id = q.incident
        JOIN project_supervisor s ON s.project = i.project AND s.enabled
        LEFT JOIN supervisor_line_repeats r ON r.incident = i.id
        WHERE (NOT ?2 OR q.urgent OR EXISTS (
            SELECT 1 FROM supervisor_incident_signals WHERE incident = i.id AND urgent))
          AND q.id = (SELECT min(id) FROM supervisor_line_pending WHERE incident = i.id)
          AND (q.kind <> 'updated' OR r.last_at IS NULL OR r.last_at + ?3 <= ?1 OR i.closed_at IS NOT NULL)
        UNION ALL
        SELECT NULL, i.id, i.project, 'repeat', max(m.level), count(*), r.escalated
        FROM supervisor_line_repeats r JOIN supervisor_incidents i ON i.id = r.incident
        JOIN project_supervisor s ON s.project = i.project AND s.enabled
        JOIN supervisor_incident_signals m ON m.incident = i.id
        WHERE r.due_at <= ?1 AND i.closed_at IS NULL AND i.acknowledged_at IS NULL
          AND coalesce(i.snoozed_until, 0) <= ?1 AND r.last_at + ?3 <= ?1
        GROUP BY i.id HAVING NOT ?2 OR max(m.urgent)
    ) SELECT pending, incident, project, kind, level, signals, escalated
      FROM ready ORDER BY level DESC, incident, pending LIMIT 1", (at, quiet, first), |r| {
        let kind = Kind::read(&r.get::<_, String>(3)?);
        let escalated: bool = r.get(6)?;
        let notice = Notice { incident: r.get(1)?, project: r.get(2)?, kind,
            level: r.get(4)?, signals: r.get(5)?, channels: if kind == Kind::Repeat && escalated { 2 } else { 1 } };
        Ok((r.get::<_, Option<i64>>(0)?, notice, escalated))
    }).optional().map_err(sql)?;
    let Some((pending, notice, escalated)) = selected else { return Ok(None) };
    let id = notice.incident;
    if let Some(row) = pending {
        tx.execute("DELETE FROM supervisor_line_pending WHERE id = ?1", [row]).map_err(sql)?;
    }
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
    Ok(Some(notice))
}
