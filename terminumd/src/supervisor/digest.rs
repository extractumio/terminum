// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-243: an exception digest from retained signals, with no judgment round.

use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;

use crate::tasks::ops::{rows, sql};
use super::{DAY, Measure, signals};

#[derive(Serialize)]
pub(super) struct Entry {
    fingerprint: String,
    detector: String,
    metric: String,
    measure: Measure,
    action: &'static str,
}

#[derive(Serialize)]
pub(super) struct Digest {
    at: i64,
    entries: Vec<serde_json::Value>,
    last_ok: Option<i64>,
}

pub(super) fn latest(tx: &Transaction, project: &str) -> Result<Option<Digest>, String> {
    tx.query_row("SELECT d.at, d.document, d.last_ok FROM supervisor_digests d
        WHERE d.project = ?1", [project], |r| Ok(Digest { at: r.get(0)?, entries: signals::decode(r, 1)?, last_ok: r.get(2)? }))
        .optional().map_err(sql)
}

pub(super) fn publish(tx: &Transaction, project: &str, at: i64) -> Result<bool, String> {
    let (enabled, previous, last_ok): (i64, Option<i64>, Option<i64>) = tx.query_row(
        "SELECT p.enabled_at, d.at, p.last_ok FROM project_supervisor p LEFT JOIN supervisor_digests d USING (project)
        WHERE p.project = ?1 AND p.enabled", [project], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(sql)?;
    let day = at.div_euclid(DAY);
    if previous.is_some_and(|p| p.div_euclid(DAY) >= day) { return Ok(false); }
    let after = previous.unwrap_or(enabled).max(enabled);
    let entries = rows(tx, "SELECT fingerprint, detector, metric, measure, state FROM (
        SELECT *, max(json_extract(scores, '$.impact'), json_extract(scores, '$.blast'),
          json_extract(scores, '$.reversibility')) AS level FROM supervisor_signals WHERE project = ?1 AND first_seen >= ?2 AND first_seen < ?3
        ) s WHERE level >= 1 AND state NOT IN ('shadow', 'muted', 'rejected')
        AND NOT EXISTS (SELECT 1 FROM supervisor_detector_lifecycle l
            WHERE l.project = s.project AND l.detector = s.detector AND l.weekly_at IS NOT NULL)
        AND coalesce((SELECT json_extract(w.pack, '$.detectors.\"' || s.detector || '\".mute.until')
            FROM supervisor_settings w WHERE w.project = s.project), 0) <= ?3
        ORDER BY level DESC, first_seen DESC, fingerprint LIMIT 5", (project, after, at), |r| {
            let action = match r.get::<_, String>(4)?.as_str() {
                "held" => "Held",
                "resolved" => "Resolved",
                "raised" | "recurred" => "Raised for review",
                _ => "Recorded for review",
            };
            Ok(Entry { fingerprint: r.get(0)?, detector: r.get(1)?, metric: r.get(2)?, measure: signals::decode(r, 3)?, action })
        })?;
    if entries.is_empty() { return Ok(false); }
    // the digest is also a report in the owner's Inbox (terminum-reports.md AC-272)
    let mut lines: Vec<String> = entries.iter().map(|e| format!("{} · {}: {} against a baseline of {} — {}", e.detector, e.metric, e.measure.value, e.measure.baseline, e.action)).collect();
    lines.push(format!("The collector's last run: {}", last_ok.map_or_else(|| "none yet".to_string(), crate::tasks::cron::stamp)));
    crate::reports::sources::digest(tx, project, &lines, at)?;
    let document = serde_json::to_string(&entries).map_err(|e| e.to_string())?;
    tx.execute("INSERT INTO supervisor_digests (project, at, document, last_ok) VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT (project) DO UPDATE SET at = excluded.at, document = excluded.document, last_ok = excluded.last_ok",
        (project, at, document, last_ok)).map_err(sql)?;
    Ok(true)
}
