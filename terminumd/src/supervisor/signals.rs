// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};


use crate::tasks::{ops::{rows, sql}, store::Store};

use super::{Evaluation, Evidence, Finding, Measure, Mode, Scores};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum State { New, Shadow, Raised, Held, Resolved, Recurred, Muted, Rejected }

#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct Signal {
    /// The fingerprint doubles as the stable record ID.
    pub fingerprint: String,
    pub project: String,
    pub detector: String,
    pub subject_kind: String,
    pub subject_id: String,
    pub metric: String,
    pub evidence: Vec<Evidence>,
    pub measure: Measure,
    pub scores: Scores,
    pub level: u8,
    pub response: &'static str,
    pub first_seen: i64,
    pub last_seen: i64,
    pub count: i64,
    pub state: State,
}

fn fingerprint(project: &str, detector: &str, found: &Finding) -> String {
    let mut hash = Sha256::new();
    for field in [project, detector, found.subject_kind, &found.subject_id, found.metric] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    crate::auth::crypto::hex(&hash.finalize())
}

struct PreparedFinding<'a> {
    key: String,
    found: &'a Finding,
    evidence: String,
    measure: String,
}

pub(super) fn record(store: &Store, evaluation: &Evaluation) -> Result<(), String> {
    let mut batches = Vec::new();
    for detector in &evaluation.detectors {
        let complete = detector.unavailable.is_empty();
        if !complete && detector.complete_metrics.is_empty() && detector.findings.is_empty() {
            continue;
        }
        let rule = detector.rule.id();
        let findings: Vec<_> = detector.findings.iter().map(|found| {
            Ok::<_, String>(PreparedFinding {
                key: fingerprint(&evaluation.project, rule, found), found,
                evidence: encode(&found.evidence)?, measure: encode(&found.measure)?,
            })
        }).collect::<Result<_, _>>()?;
        let keys = encode(&findings.iter().map(|f| &f.key).collect::<Vec<_>>())?;
        batches.push((detector, findings, keys, encode(&detector.scores)?, encode(&detector.complete_metrics)?));
    }
    if batches.is_empty() {
        return Ok(());
    }
    store.with(|tx| {
        let mut upsert = tx.prepare_cached("INSERT INTO supervisor_signals
            (fingerprint, project, detector, subject_kind, subject_id, metric, evidence, measure, scores, first_seen, last_seen, count, state)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 1, ?11)
            ON CONFLICT (fingerprint) DO UPDATE SET evidence = excluded.evidence, measure = excluded.measure,
            scores = excluded.scores, last_seen = excluded.last_seen, count = count + 1,
            state = CASE
                WHEN state IN ('held', 'muted', 'rejected') THEN state
                WHEN excluded.state = 'shadow' THEN 'shadow'
                WHEN state = 'shadow' THEN 'new'
                WHEN state = 'resolved' THEN 'recurred'
                ELSE state END").map_err(sql)?;
        for (detector, findings, keys, scores, metrics) in batches {
            let rule = detector.rule.id();
            let advanced = tx.execute("INSERT INTO supervisor_evaluations VALUES (?1, ?2, ?3)
                ON CONFLICT (project, detector) DO UPDATE SET evaluated_at = excluded.evaluated_at
                WHERE excluded.evaluated_at > supervisor_evaluations.evaluated_at", (&evaluation.project, rule, evaluation.at)).map_err(sql)?;
            if advanced == 0 {
                continue;
            }
            for row in findings {
                upsert.execute(params![row.key, evaluation.project, rule, row.found.subject_kind,
                    row.found.subject_id, row.found.metric, row.evidence, row.measure, scores, evaluation.at,
                    if detector.mode == Mode::Shadow { "shadow" } else { "new" }]).map_err(sql)?;
            }
            tx.execute("UPDATE supervisor_signals SET state = 'resolved'
                WHERE project = ?1 AND detector = ?2 AND state IN ('new', 'shadow', 'raised', 'recurred')
                AND fingerprint NOT IN (SELECT value FROM json_each(?3))
                AND (?4 OR metric IN (SELECT value FROM json_each(?5)))",
                (&evaluation.project, rule, keys, detector.unavailable.is_empty(), metrics)).map_err(sql)?;
        }
        Ok(())
    })
}

fn encode(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("Could not encode Supervisor signal: {e}"))
}

fn decode<T: serde::de::DeserializeOwned>(row: &rusqlite::Row<'_>, col: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&row.get::<_, String>(col)?)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e)))
}

pub(super) fn list(tx: &Transaction, project: &str) -> Result<Vec<Signal>, String> {
    rows(tx, "SELECT fingerprint, detector, subject_kind, subject_id, metric, evidence, measure, scores,
        first_seen, last_seen, count, json_quote(state) FROM supervisor_signals WHERE project = ?1
        ORDER BY detector, subject_kind, subject_id, metric", [project], |r| {
        let scores: Scores = decode(r, 7)?;
        let level = scores.level();
        Ok(Signal {
            fingerprint: r.get(0)?, project: project.into(), detector: r.get(1)?, subject_kind: r.get(2)?,
            subject_id: r.get(3)?, metric: r.get(4)?, evidence: decode(r, 5)?, measure: decode(r, 6)?,
            scores, level, response: ["R0", "R2", "R4", "R5"][usize::from(level)],
            first_seen: r.get(8)?, last_seen: r.get(9)?, count: r.get(10)?, state: decode(r, 11)?,
        })
    })
}
