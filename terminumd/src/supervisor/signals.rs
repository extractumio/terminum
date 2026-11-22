// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};


use crate::tasks::{ops::{rows, sql}, store::Store};

use super::impact::Assessment;
use super::{Evaluation, Evidence, Finding, Measure, Mode, Scores, incidents};

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
    pub(super) impact_basis: Assessment,
    pub level: u8,
    pub response: &'static str,
    pub first_seen: i64,
    pub last_seen: i64,
    pub count: i64,
    pub state: State,
}

fn fingerprint(project: &str, detector: &str, (kind, id, metric): (&str, &str, &str)) -> String {
    let mut hash = Sha256::new();
    for field in [project, detector, kind, id, metric] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    crate::auth::crypto::hex(&hash.finalize())
}

#[derive(Serialize, Deserialize)]
struct RecordedScores {
    #[serde(flatten)]
    scores: Scores,
    #[serde(default)]
    impact_basis: Assessment,
}

struct PreparedFinding<'a> {
    key: String,
    found: &'a Finding,
    evidence: String,
    measure: String,
    scores: String,
}

const ACTIVE: &str = "state IN ('new', 'shadow', 'raised', 'recurred')";

struct PreparedRule<'a> {
    checked: &'a super::Checked,
    findings: Vec<PreparedFinding<'a>>,
    keys: String,
    complete_metrics: String,
    measured: String,
}

pub(super) struct Prepared<'a> {
    evaluation: &'a Evaluation,
    batches: Vec<PreparedRule<'a>>,
}

pub(super) fn record(store: &Store, evaluation: &Evaluation) -> Result<(), String> {
    let prepared = prepare(evaluation)?;
    if prepared.batches.is_empty() {
        return Ok(());
    }
    store.with(|tx| prepared.write(tx, false))
}

pub(super) fn prepare(evaluation: &Evaluation) -> Result<Prepared<'_>, String> {
    let mut batches = Vec::new();
    for detector in &evaluation.detectors {
        let complete = detector.unavailable.is_empty();
        if !complete && detector.complete_metrics.is_empty() && detector.findings.is_empty() {
            continue;
        }
        let rule = detector.rule.id();
        let findings: Vec<_> = detector.findings.iter().map(|found| {
            Ok::<_, String>(PreparedFinding {
                key: fingerprint(&evaluation.project, rule, found.key()), found,
                evidence: encode(&found.evidence)?, measure: encode(&found.measure)?,
                scores: encode(&RecordedScores { scores: found.scores, impact_basis: found.assessment.clone() })?,
            })
        }).collect::<Result<_, _>>()?;
        let keys = encode(&findings.iter().map(|f| &f.key).collect::<Vec<_>>())?;
        let measured = detector.measured.as_deref().unwrap_or_default().iter()
            .map(|(kind, id, metric)| fingerprint(&evaluation.project, rule, (kind, id, metric))).collect::<Vec<_>>();
        batches.push(PreparedRule { checked: detector, findings, keys,
            complete_metrics: encode(&detector.complete_metrics)?, measured: encode(&measured)? });
    }
    Ok(Prepared { evaluation, batches })
}

impl Prepared<'_> {
    pub(super) fn write(self, tx: &Transaction, seed_attachment: bool) -> Result<(), String> {
        let Self { evaluation, batches } = self;
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
                ELSE state END RETURNING state NOT IN ('muted', 'rejected')").map_err(sql)?;
        for PreparedRule { checked, findings, keys, complete_metrics, measured } in batches {
            let rule = checked.rule.id();
            let advanced = tx.execute("INSERT INTO supervisor_evaluations VALUES (?1, ?2, ?3)
                ON CONFLICT (project, detector) DO UPDATE SET evaluated_at = excluded.evaluated_at
                WHERE excluded.evaluated_at > supervisor_evaluations.evaluated_at", (&evaluation.project, rule, evaluation.at)).map_err(sql)?;
            if advanced == 0 {
                continue;
            }
            for row in findings {
                let accepted: bool = upsert.query_row(params![row.key, evaluation.project, rule, row.found.subject_kind,
                    row.found.subject_id, row.found.metric, row.evidence, row.measure, row.scores, evaluation.at,
                    if checked.mode == Mode::Shadow { "shadow" } else { "new" }], |r| r.get(0)).map_err(sql)?;
                if checked.mode == Mode::Live && accepted {
                    incidents::observe(tx, incidents::Observation {
                        project: &evaluation.project, rule: checked.rule, fingerprint: &row.key,
                        finding: row.found, evidence: &row.evidence, at: evaluation.at,
                    })?;
                }
                if seed_attachment && checked.mode == Mode::Live && row.found.scores.impact >= 2 {
                    tx.execute("INSERT OR IGNORE INTO supervisor_attach_seeds (fingerprint, seeded_at)
                        VALUES (?1, ?2)", (&row.key, evaluation.at)).map_err(sql)?;
                }
            }
            let resolved = tx.execute(&format!("UPDATE supervisor_signals SET state = 'resolved'
                WHERE project = ?1 AND detector = ?2 AND {ACTIVE}
                AND fingerprint NOT IN (SELECT value FROM json_each(?3))
                AND (?4 OR metric IN (SELECT value FROM json_each(?5)) OR fingerprint IN (SELECT value FROM json_each(?6)))"),
                (&evaluation.project, rule, keys, checked.unavailable.is_empty() && checked.measured.is_none(),
                    complete_metrics, measured)).map_err(sql)?;
            if resolved != 0 {
                incidents::clear_resolved(tx, &evaluation.project, checked.rule, evaluation.at)?;
            }
        }
        Ok(())
    }
}

pub(super) fn resolve_rule(tx: &Transaction, project: &str, rule: super::Rule, at: i64) -> Result<(), String> {
    let resolved = tx.execute(&format!("UPDATE supervisor_signals SET state = 'resolved'
        WHERE project = ?1 AND detector = ?2 AND {ACTIVE}"), (project, rule.id())).map_err(sql)?;
    if resolved != 0 {
        incidents::clear_resolved(tx, project, rule, at)?;
    }
    Ok(())
}

fn encode(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_string(value).map_err(|e| format!("Could not encode Supervisor signal: {e}"))
}

fn decode<T: serde::de::DeserializeOwned>(row: &rusqlite::Row<'_>, col: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&row.get::<_, String>(col)?)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e)))
}

const COLUMNS: &str = "fingerprint, detector, subject_kind, subject_id, metric, evidence, measure, scores,
    first_seen, last_seen, count, json_quote(state)";

fn read(project: &str, r: &rusqlite::Row<'_>) -> rusqlite::Result<Signal> {
    let RecordedScores { scores, impact_basis } = decode(r, 7)?;
    let level = scores.level();
    let count = r.get(10)?;
    Ok(Signal {
        fingerprint: r.get(0)?, project: project.into(), detector: r.get(1)?, subject_kind: r.get(2)?,
        subject_id: r.get(3)?, metric: r.get(4)?, evidence: decode(r, 5)?, measure: decode(r, 6)?,
        scores, impact_basis, level, response: scores.response(count > 1),
        first_seen: r.get(8)?, last_seen: r.get(9)?, count, state: decode(r, 11)?,
    })
}

pub(super) fn list(tx: &Transaction, project: &str) -> Result<Vec<Signal>, String> {
    rows(tx, &format!("SELECT {COLUMNS} FROM supervisor_signals WHERE project = ?1
        ORDER BY detector, subject_kind, subject_id, metric"), [project], |r| read(project, r))
}

pub(super) fn page(tx: &Transaction, project: &str, after: Option<&str>, limit: usize) -> Result<Vec<Signal>, String> {
    rows(tx, &format!("SELECT {COLUMNS} FROM supervisor_signals WHERE project = ?1
        AND (?2 IS NULL OR (first_seen, fingerprint) < (
            SELECT first_seen, fingerprint FROM supervisor_signals WHERE fingerprint = ?2 AND project = ?1
        )) ORDER BY first_seen DESC, fingerprint DESC LIMIT ?3"),
        (project, after, limit as i64), |r| read(project, r))
}
