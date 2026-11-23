// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};


use crate::tasks::{ops::{rows, sql}, store::Store};

use super::impact::Assessment;
use super::{Evaluation, Evidence, Finding, Measure, Mode, Scores, incidents};

pub(super) type Observations = std::collections::BTreeMap<(String, String, String), Option<(f64, super::Comparison)>>;

impl super::Comparison {
    pub(super) fn fires(self, value: f64, limit: f64) -> bool {
        match self {
            Self::Above => value > limit,
            Self::AtLeast => value >= limit,
            Self::Below => value < limit,
            Self::Deviation(target) => (value - target).abs() > limit,
        }
    }
}

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
    pub(super) charter_breach: bool,
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
pub(super) struct RecordedScores {
    #[serde(flatten)]
    pub scores: Scores,
    #[serde(default)]
    pub impact_basis: Assessment,
    #[serde(default)]
    pub charter_breach: bool,
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
                scores: encode(&RecordedScores { scores: found.scores, impact_basis: found.assessment.clone(), charter_breach: found.charter_breach })?,
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
            let complete = checked.unavailable.is_empty() && checked.measured.is_none();
            let fired: std::collections::HashSet<_> = findings.iter().map(|f| f.key.as_str()).collect();
            let known = rows(tx, "SELECT y.fingerprint, y.threshold, s.subject_kind, s.subject_id, s.metric, y.clear_at, y.clear_seen
                FROM supervisor_problem_signals y JOIN supervisor_signals s USING (fingerprint)
                WHERE s.project = ?1 AND s.detector = ?2", (&evaluation.project, rule),
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?, (r.get::<_, String>(2)?, r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?), r.get::<_, Option<i64>>(5)?, r.get::<_, Option<i64>>(6)?)))?;
            for (key, threshold, subject, previous, sampled) in known {
                let clear = checked.mode == Mode::Live && !fired.contains(key.as_str()) &&
                    match checked.observations.get(&subject) {
                        Some(reading) => reading.is_some_and(|(value, compare)| !compare.fires(value, threshold)),
                        None => complete || checked.complete_metrics.iter().any(|m| *m == subject.2),
                    };
                let since = clear.then(|| previous.filter(|_| sampled.is_some_and(|t| evaluation.at - t <= 30 * 60))
                    .unwrap_or(evaluation.at));
                tx.execute("UPDATE supervisor_problem_signals SET clear_at = ?2, clear_seen = ?3
                    WHERE fingerprint = ?1 AND (clear_at, clear_seen) IS NOT (?2, ?3)",
                    (&key, since, clear.then_some(evaluation.at))).map_err(sql)?;
            }
            for row in findings {
                let accepted: bool = upsert.query_row(params![row.key, evaluation.project, rule, row.found.subject_kind,
                    row.found.subject_id, row.found.metric, row.evidence, row.measure, row.scores, evaluation.at,
                    if checked.mode == Mode::Shadow { "shadow" } else { "new" }], |r| r.get(0)).map_err(sql)?;
                if checked.mode == Mode::Live && accepted {
                    let week = evaluation.at.div_euclid(super::WEEK);
                    if tx.execute("INSERT OR IGNORE INTO supervisor_signal_weeks VALUES (?1, ?2)",
                        (&row.key, week)).map_err(sql)? != 0
                    {
                        tx.execute("DELETE FROM supervisor_signal_weeks WHERE fingerprint = ?1 AND week < ?2",
                            (&row.key, week - 7)).map_err(sql)?;
                    }
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

pub(super) fn decode<T: serde::de::DeserializeOwned>(row: &rusqlite::Row<'_>, col: usize) -> rusqlite::Result<T> {
    serde_json::from_str(&row.get::<_, String>(col)?)
        .map_err(|e| rusqlite::Error::FromSqlConversionFailure(col, rusqlite::types::Type::Text, Box::new(e)))
}

const COLUMNS: &str = "fingerprint, detector, subject_kind, subject_id, metric, evidence, measure, scores,
    first_seen, last_seen, count, json_quote(state)";

fn read(project: &str, r: &rusqlite::Row<'_>) -> rusqlite::Result<Signal> {
    let RecordedScores { scores, impact_basis, charter_breach } = decode(r, 7)?;
    let level = scores.level();
    let count = r.get(10)?;
    Ok(Signal {
        fingerprint: r.get(0)?, project: project.into(), detector: r.get(1)?, subject_kind: r.get(2)?,
        subject_id: r.get(3)?, metric: r.get(4)?, evidence: decode(r, 5)?, measure: decode(r, 6)?,
        scores, impact_basis, charter_breach, level, response: scores.response(count > 1),
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


pub(super) fn get(tx: &Transaction, project: &str, key: &str) -> Result<Signal, String> {
    tx.query_row(&format!("SELECT {COLUMNS} FROM supervisor_signals WHERE project = ?1 AND fingerprint = ?2"),
        (project, key), |r| read(project, r)).optional().map_err(sql)?.ok_or("No such signal in this project.".into())
}


/// A point event is retained by case and never resolves unrelated failures.
pub(super) fn round_failed(tx: &Transaction, project: &str, id: &str, metric: &str, value: f64, at: i64) -> Result<(), String> {
    diagnostic(tx, project, "self.round_failed", ("round", id, metric),
        &[Evidence::SupervisorRounds(id.into())], (value, 0.), at)
}

/// A code-owned diagnostic carries typed evidence and a fixed metric.
pub(super) fn diagnostic(tx: &Transaction, project: &str, detector: &str, subject: (&str, &str, &str),
    evidence: &[Evidence], (value, threshold): (f64, f64), at: i64) -> Result<(), String> {
    let (kind, id, metric) = subject;
    let key = fingerprint(project, detector, subject);
    let evidence = encode(&evidence)?;
    let measure = encode(&Measure { value, baseline: 0., threshold, window_start: at, window_end: at })?;
    let scores = encode(&RecordedScores { scores: Scores { impact: 1, ..Scores::default() }, impact_basis: Default::default(), charter_breach: false })?;
    tx.execute("INSERT INTO supervisor_signals (fingerprint, project, detector, subject_kind, subject_id,
        metric, evidence, measure, scores, first_seen, last_seen, count, state)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 1, 'raised')
        ON CONFLICT (fingerprint) DO UPDATE SET measure = excluded.measure, evidence = excluded.evidence,
            last_seen = excluded.last_seen, count = count + 1, state = 'raised'
        WHERE (state, evidence, json_extract(measure, '$.value'), json_extract(measure, '$.threshold'))
            IS NOT (excluded.state, excluded.evidence, ?11, ?12)",
        params![key, project, detector, kind, id, metric, evidence, measure, scores, at, value, threshold]).map_err(sql)?;
    Ok(())
}
