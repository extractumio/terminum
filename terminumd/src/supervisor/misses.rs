// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-255: freeze what was known when an independent source reported a problem.

use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::json;

use crate::tasks::{ops::{Task, rows, sql}, store::HUMAN};

use super::{reflection, rounds};

pub(crate) fn owner_bug(tx: &Transaction, author: &str, task: &Task, at: i64) -> Result<(), String> {
    if author != HUMAN || task.kind.as_deref() != Some("Bug") || !matches!(task.severity.as_str(), "P0" | "P1") {
        return Ok(());
    }
    if rounds::enabled(tx, &task.project)?.is_none() { return Ok(()); }
    let keys = rows(tx, "SELECT id FROM tasks WHERE id IN (
        SELECT id FROM tasks WHERE id = ?1 UNION SELECT parent FROM tasks WHERE id = ?1
        UNION SELECT other FROM task_links WHERE task = ?1 AND kind = 'follow_ups'
        UNION SELECT task FROM task_links WHERE other = ?1 AND kind = 'follow_ups')
        AND project = ?2", (&task.key, &task.project), |r| r.get::<_, String>(0))?;
    observe(tx, &task.project, "owner_bug", ("task", &task.key), (&task.key, &keys), (at, at))
}

pub(crate) fn target_red(tx: &Transaction, project: &str, target: &str, commit: &str, task: &str, at: i64) -> Result<(), String> {
    if rounds::enabled(tx, project)?.is_none() { return Ok(()); }
    observe(tx, project, "target_red", ("target", target), (commit, &[task.into()]), (at, at))
}

pub(super) fn watchdog(tx: &Transaction, project: &str, at: i64, due: i64) -> Result<(), String> {
    observe(tx, project, "watchdog", ("project", project), (&due.to_string(), &[]), (at, due))
}

fn observe(tx: &Transaction, project: &str, source: &str, (kind, subject): (&str, &str),
    (occurrence, task_keys): (&str, &[String]), (at, independent_at): (i64, i64)) -> Result<(), String>
{
    let recorded: bool = tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_miss_checks
        WHERE project = ?1 AND source = ?2 AND subject_kind = ?3 AND subject_id = ?4 AND occurrence = ?5)",
        (project, source, kind, subject, occurrence), |r| r.get(0)).map_err(sql)?;
    if recorded { return Ok(()); }
    // A live incident or an explicit raise is evidence of delivery to the owner.
    // Record suppression based on how the matching signal stood at observation time.
    let prior = tx.query_row("WITH candidates AS (
        SELECT * FROM supervisor_signals WHERE project = ?1 AND subject_kind = ?2 AND subject_id = ?3
        UNION SELECT s.* FROM json_each(?5) k JOIN supervisor_signals s
          ON s.project = ?1 AND s.subject_kind = 'task' AND s.subject_id = k.value
    ), matched AS (
        SELECT s.fingerprint, s.state, s.detector, s.first_seen,
          coalesce((SELECT min(e.at) FROM json_each(s.evidence) proof JOIN events e
              ON json_extract(proof.value,'$.table') = 'events' AND json_extract(proof.value,'$.id') = e.id
              JOIN tasks t ON t.id = e.task AND t.project = ?1 WHERE e.at <= s.first_seen), s.first_seen) AS first_proof,
          max(json_extract(s.scores,'$.impact'), json_extract(s.scores,'$.blast'),
              json_extract(s.scores,'$.reversibility')) AS level,
          coalesce((SELECT min(e.at) FROM supervisor_incident_events e JOIN supervisor_incidents i ON i.id = e.incident
            WHERE i.project = ?1 AND e.fingerprint = s.fingerprint AND e.kind IN ('signal','updated','continued') AND e.at < ?4 AND e.at >= coalesce(s.episode_raised_at, s.last_seen)),
            CASE WHEN s.episode_raised_at < ?4 THEN s.episode_raised_at END) AS raised
        FROM candidates s
        WHERE s.first_seen < ?4
          AND s.detector <> 'self.miss' AND (?6 <> 'watchdog' OR s.detector = 'self.collector_dead')
          AND s.state <> 'resolved'
    ) SELECT fingerprint, state, min(first_proof) OVER (), raised, level, detector FROM matched
      ORDER BY raised IS NULL, first_seen, fingerprint LIMIT 1", (project, kind, subject, at, super::signals::encode(&task_keys)?, source),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?,
            r.get::<_, Option<i64>>(3)?, r.get::<_, Option<i64>>(4)?, r.get::<_, String>(5)?))).optional().map_err(sql)?;
    let (signal, state, first, raised, level) = match prior {
        Some((signal, mut state, first, raised, level, detector)) => {
            if state == "shadow" && let Ok(rule) = super::Rule::named(&detector)
                && super::settings::read(tx, project)?.settings.muted(rule, at)
            {
                state = "muted".into();
            }
            (Some(signal), Some(state), first.min(independent_at), raised, level)
        }

        None => (None, None, independent_at, None, (source == "watchdog").then_some(3)),
    };
    let inserted = tx.execute("INSERT OR IGNORE INTO supervisor_miss_checks
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
        params![project, source, kind, subject, occurrence, at, first, signal, state, raised, level]).map_err(sql)?;
    if inserted != 0 && raised.is_none() {
        reflection::enqueue(tx, project, ("miss", &format!("{source}/{subject}/{occurrence}"), signal.as_deref()), "miss",
            &json!({"source":source,"subject_kind":kind,"subject_id":subject,"first_evidence_at":first,
                "observed_at":at,"signal_state":state,"raised":false}), at)?;
    }
    Ok(())
}
