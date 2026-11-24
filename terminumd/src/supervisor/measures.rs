// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-256: the scorecard is calculated from retained observations, never from a round's report.

use rusqlite::Transaction;
use serde_json::{Value, json};

use crate::tasks::ops::{rows, sql};

use super::{DAY, WEEK, governor::Spending, outcomes::LATEST, rate_intervals};

fn share(numerator: i64, sample: i64) -> rate_intervals::Rate {
    rate_intervals::share(numerator as usize, sample as usize, None)
}

pub(super) fn page_dispatched(tx: &Transaction, project: &str, at: i64) -> Result<(), String> {
    tx.execute("INSERT INTO supervisor_page_dispatches (project, at)
        SELECT id, ?2 FROM projects WHERE id = ?1", (project, at)).map(drop).map_err(sql)
}

/// A report verdict `v` that is its item's latest as of the SQL time expression `at`.
pub(super) fn latest_report_verdict(at: &str) -> String {
    format!("v.id = (SELECT max(last.id) FROM supervisor_verdicts last
        WHERE last.task = v.task AND last.fingerprint = v.fingerprint AND last.at <= {at})")
}

/// The fix observation `f` of problem `p` saw no recurrence up to the SQL time expression `at`.
pub(super) fn stayed_resolved(at: &str) -> String {
    format!("NOT EXISTS (SELECT 1 FROM supervisor_problem_events e WHERE e.problem = p.id
        AND e.state = 'recurred' AND e.at > f.confirmed_at AND e.at <= {at})")
}

fn decisions(tx: &Transaction, project: &str) -> Result<(Value, Vec<Value>), String> {
    let groups = rows(tx, &format!("SELECT a.kind, a.detector, a.response,
        count(*) FILTER (WHERE o.verdict = 'right'), count(*) FILTER (WHERE o.verdict = 'wrong'), count(*)
        FROM supervisor_actions a {LATEST} WHERE a.project = ?1
        GROUP BY a.kind, a.detector, a.response ORDER BY a.kind, a.detector, a.response"), [project], |r|
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?, r.get::<_, i64>(4)?, r.get::<_, i64>(5)?)))?;
    let (mut total_right, mut total_judged, mut total) = (0, 0, 0);
    let by_rule = groups.into_iter().map(|(kind, detector, response, right, wrong, n)| {
        let judged = right + wrong;
        total_right += right;
        total_judged += judged;
        total += n;
        json!({"kind":kind,"detector":detector,"response":response,"right":right,
            "pending":n-judged,"rate":share(right, judged)})
    }).collect();
    Ok((json!({"right":total_right,"pending":total-total_judged,
        "rate":share(total_right, total_judged)}), by_rule))
}

// AC-217: it runs each pass in the store's write turn, so CROSS JOIN keeps SQLite starting from the project's few
// problems rather than from every task of the project.
const LANDED: &str = "WITH landed AS (
    SELECT l.problem, max(e.at) AS landed_at FROM supervisor_problems p
    CROSS JOIN supervisor_problem_tasks l ON l.problem = p.id
    CROSS JOIN tasks t ON t.id = l.task AND t.project = p.project
    CROSS JOIN events e ON e.task = t.id AND e.author = 'dispatcher'
      AND e.kind = 'comment' AND json_extract(e.data,'$.left') = 'landed' AND e.at <= ?2
    WHERE p.project = ?1
    GROUP BY l.problem
)";

pub(super) fn observe_resolutions(tx: &Transaction, project: &str, at: i64) -> Result<(), String> {
    tx.execute(&format!("{LANDED}
        INSERT OR IGNORE INTO supervisor_fix_observations (problem, landed_at, confirmed_at)
        SELECT l.problem, l.landed_at, ?2 FROM landed l
        WHERE NOT EXISTS (SELECT 1 FROM supervisor_fix_observations f WHERE f.problem = l.problem AND f.landed_at = l.landed_at)
          AND EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.problem = l.problem)
          AND NOT EXISTS (SELECT 1 FROM supervisor_problem_signals y WHERE y.problem = l.problem AND
             (y.clear_at IS NULL OR y.clear_seen IS NULL OR y.clear_seen <= ?2 - ?4
              OR max(y.clear_at, l.landed_at) + y.weeks * ?3 > y.clear_seen))"),
        (project, at, WEEK, super::register::FRESH_EVIDENCE)).map(drop).map_err(sql)
}

fn resolved(tx: &Transaction, project: &str, at: i64) -> Result<(i64, i64, i64, i64), String> {
    tx.query_row(&format!("{LANDED}, checked AS (
        SELECT f.confirmed_at IS NOT NULL AS clear, p.state = 'recurred' OR EXISTS (
            SELECT 1 FROM supervisor_problem_events e WHERE e.problem = l.problem AND e.state = 'recurred'
                AND e.at > f.confirmed_at AND e.at <= ?2
        ) AS recurred
        FROM landed l JOIN supervisor_problems p ON p.id = l.problem
        LEFT JOIN supervisor_fix_observations f ON f.problem = l.problem AND f.landed_at = l.landed_at AND f.confirmed_at <= ?2
    ) SELECT count(*) FILTER (WHERE clear AND NOT recurred), count(*),
        count(*) FILTER (WHERE NOT clear AND NOT recurred), count(*) FILTER (WHERE recurred) FROM checked"),
        (project, at), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))).map_err(sql)
}

fn overrides(tx: &Transaction, project: &str) -> Result<(i64, i64), String> {
    tx.query_row("SELECT count(*) FILTER (WHERE
        (a.kind = 'hold' AND EXISTS (SELECT 1 FROM supervisor_holds h
            WHERE h.id = a.source AND h.released_by = 'human' AND h.released_at BETWEEN a.at AND a.at + ?2))
        OR EXISTS (SELECT 1 FROM supervisor_task_overrides v
            WHERE v.project = ?1 AND v.action = a.id AND v.at BETWEEN a.at AND a.at + ?2)), count(*)
        FROM supervisor_actions a WHERE a.project = ?1",
        (project, DAY), |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)
}

fn delays(tx: &Transaction, project: &str) -> Result<Vec<Value>, String> {
    // An actual event or task date supports a delay. Evaluation window bounds alone do not.
    rows(tx, "WITH evidence AS (
        SELECT s.fingerprint, coalesce(s.episode_raised_at, (
          SELECT min(i.at) FROM supervisor_incident_events i JOIN supervisor_incidents p ON p.id = i.incident
          WHERE p.project = s.project AND i.fingerprint = s.fingerprint AND i.kind IN ('signal','updated','continued')
        )) AS raised,
          max(json_extract(s.scores,'$.impact'),json_extract(s.scores,'$.blast'),
              json_extract(s.scores,'$.reversibility')) AS level,
          min(coalesce(e.at,t.created_at)) AS first_at
        FROM supervisor_signals s JOIN json_each(s.evidence) proof
        LEFT JOIN events e ON json_extract(proof.value,'$.table') = 'events' AND e.id = json_extract(proof.value,'$.id')
        LEFT JOIN tasks t ON json_extract(proof.value,'$.table') = 'tasks' AND t.id = json_extract(proof.value,'$.id')
        WHERE s.project = ?1 AND s.state IN ('new','raised','held','recurred','resolved')
        GROUP BY s.fingerprint
    ), supported AS (
        SELECT level, raised - first_at AS seconds FROM evidence WHERE level IS NOT NULL AND first_at <= raised
        UNION ALL SELECT 3, observed_at - first_evidence_at FROM supervisor_miss_checks
            WHERE project = ?1 AND source = 'watchdog' AND first_evidence_at <= observed_at
    ) SELECT level, count(*), avg(seconds) FROM supported GROUP BY level ORDER BY level",
        [project], |r| Ok(json!({"level":r.get::<_, u8>(0)?,"sample":r.get::<_, i64>(1)?,
            "seconds":r.get::<_, f64>(2)?})))
}

fn owner_load(tx: &Transaction, project: &str, at: i64) -> Result<(i64, i64, i64, i64, f64), String> {
    tx.query_row("SELECT
        (SELECT count(*) FROM supervisor_actions WHERE project = ?1 AND kind = 'card' AND at BETWEEN ?2 AND ?3),
        (SELECT count(*) FROM supervisor_page_dispatches WHERE project = ?1 AND at BETWEEN ?2 AND ?3),
        count(*), count(v.active_seconds), coalesce(sum(v.active_seconds) / 60.0, 0)
        FROM supervisor_verdicts v JOIN supervisor_review_reports r USING (task)
        WHERE r.project = ?1 AND v.at BETWEEN ?2 AND ?3", (project, at - WEEK + 1, at),
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).map_err(sql)
}

pub(super) fn read(tx: &Transaction, project: &str, at: i64, spending: &Spending) -> Result<Value, String> {
    let (judged, by_rule) = decisions(tx, project)?;
    let (stayed, landed, watching, recurred) = resolved(tx, project, at)?;
    let (overridden, actions) = overrides(tx, project)?;
    let (misses, checked): (i64, i64) = tx.query_row("SELECT count(*) FILTER (WHERE raised_at IS NULL), count(*)
        FROM supervisor_miss_checks WHERE project = ?1 AND observed_at <= ?2", (project, at),
        |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
    let recent_misses = rows(tx, "SELECT m.source, coalesce(t.slug, m.subject_id), m.observed_at, m.first_evidence_at, m.signal, m.signal_state
        FROM supervisor_miss_checks m LEFT JOIN tasks t ON m.subject_kind = 'task' AND t.id = m.subject_id AND t.project = m.project
        WHERE m.project = ?1 AND m.observed_at <= ?2 AND m.raised_at IS NULL
        ORDER BY m.observed_at DESC, m.source, m.subject_id, m.occurrence LIMIT 20", (project, at), |r|
        Ok(json!({"source":r.get::<_, String>(0)?,"subject":r.get::<_, String>(1)?,"observed_at":r.get::<_, i64>(2)?,
            "first_evidence_at":r.get::<_, i64>(3)?,"signal":r.get::<_, Option<String>>(4)?,
            "signal_state":r.get::<_, Option<String>>(5)?})))?;
    let (refused, findings): (i64, i64) = tx.query_row("SELECT coalesce(sum(refused),0),
        coalesce(sum(refused),0) + count(finding) FROM supervisor_rounds q WHERE project = ?1
        AND NOT EXISTS (SELECT 1 FROM supervisor_reflections f WHERE f.round = q.id)
        AND json_extract(case_file,'$.kind') IN ('investigation','verification')", [project],
        |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
    let (useful, rated): (i64, i64) = tx.query_row(&format!("SELECT count(*) FILTER (WHERE v.verdict = 'useful'), count(*)
        FROM supervisor_verdicts v JOIN supervisor_review_reports p USING(task)
        WHERE p.project = ?1 AND v.at <= ?2 AND {}", latest_report_verdict("?2")), (project, at),
        |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
    let calibration = rows(tx, &format!("SELECT round(json_extract(q.finding,'$.confidence') * 10) / 10, count(*),
        count(*) FILTER (WHERE o.verdict = 'right') FROM supervisor_actions a {LATEST}
        JOIN supervisor_rounds q ON q.run = a.source AND q.project = a.project
        WHERE a.project = ?1 AND a.kind = 'cause' AND o.verdict IN ('right','wrong')
          AND json_type(q.finding,'$.confidence') IN ('integer','real') GROUP BY 1 ORDER BY 1"), [project], |r| {
        let (confidence, n, right): (f64, i64, i64) = (r.get(0)?, r.get(1)?, r.get(2)?);
        Ok(json!({"confidence":confidence,"sample":n,
            "rate":(n >= super::spend::MIN_RATE_SAMPLE as i64).then(|| share(right, n))}))
    })?;
    let (cards, pages, presses, timed, minutes) = owner_load(tx, project, at)?;
    let weekly_resolved: i64 = tx.query_row(&format!("SELECT count(DISTINCT p.id) FROM supervisor_problems p
        JOIN supervisor_fix_observations f ON f.problem = p.id
        WHERE p.project = ?1 AND f.confirmed_at BETWEEN ?2 AND ?3 AND {}", stayed_resolved("?3")),
        (project, at - WEEK + 1, at), |r| r.get(0)).map_err(sql)?;
    let judgment = &spending.judgment;
    let fixes = &spending.fixes;
    let per = |value: f64| (weekly_resolved > 0).then(|| value / weekly_resolved as f64);
    Ok(json!({
        "decisions":judged,"by_rule":by_rule,
        "resolved":{"count":stayed,"watching":watching,"recurred":recurred,"rate":share(stayed, landed)},
        "misses":{"count":misses,"sample":checked,"recent":recent_misses},
        "delays":delays(tx, project)?,
        "overrides":{"count":overridden,"rate":share(overridden, actions)},
        "refusals":{"count":refused,"sample":findings},
        "usefulness":{"count":useful,"rate":share(useful, rated)},
        "calibration":calibration,
        "judgment":{"tokens":judgment.tokens,"sample":judgment.sampled_runs,"per_resolution":per(judgment.tokens as f64)},
        "fixes":{"dollars":fixes.dollars,"sample":fixes.priced_runs,"unpriced":fixes.unpriced_runs,
            "per_resolution":(fixes.unpriced_runs == 0).then(|| per(fixes.dollars)).flatten()},
        "weekly_resolved":weekly_resolved,
        "owner_load":{"cards":cards,"pages":pages,"presses":presses,"timed":timed,"verdict_minutes":(timed > 0).then_some(minutes)}
    }))
}
