// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-212 and AC-261: typed integration returns, Verify outcomes and comparable task diffs.

use std::collections::{BTreeMap, BTreeSet};

use crate::tasks::ops::rows;

use super::{Comparison, Evidence, Measurements, Query, Reading, Threshold, catalog, percentile,
    flow::{RETURNED, events, project_events}, impact::{Source, Unit}, probe_rules::{self, ARCH_MISSING, INTEGRATION_OFF}};

const VERIFY: &str = "A Verify verdict is one check's result, in no weekly project total; the rework it causes is sized by flow.returned and flow.rerun_after_done.";

pub(super) fn returned(q: &Query<'_, '_>) -> Result<Measurements, String> {
    if !catalog::integration_on(q.tx, q.project)? { return Ok(Measurements::missing(INTEGRATION_OFF)); }
    let returns = rows(q.tx, &format!("{} SELECT id, task, at FROM project_events e WHERE at >= ?3 AND {RETURNED}", project_events()),
        (q.project, q.window.end, q.window.before), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?;
    let mut subjects: [BTreeSet<&str>; 2] = Default::default();
    for (_, task, at) in &returns { subjects[usize::from(q.window.current(*at))].insert(task); }
    let [old, current] = subjects.map(|tasks| tasks.len() as f64);
    let mut out = Measurements::default();
    out.readings.push(Reading::new("project", q.project, "tasks_returned", current,
        returns.into_iter().map(|(id, _, _)| events(id)).collect(), Source::Count(Unit::RunEvents)).history(vec![old]));
    let reds = rows(q.tx, "SELECT r.batch, r.round FROM integration_rounds r JOIN integration_batches b ON b.id = r.batch
        WHERE b.project = ?1 AND r.ended BETWEEN ?2 AND ?3 AND r.result = 'red'",
        (q.project, q.window.start, q.window.end), |r| Ok(Evidence::IntegrationRound { batch: r.get(0)?, round: r.get(1)? }))?;
    out.readings.push(Reading::new("project", q.project, "red_integration_rounds", reds.len() as f64, reds,
        Source::Unsized("Red rounds are integration batches, in no weekly project total; the tasks they return are sized by tasks_returned."))
        .fixed(Threshold::Absolute(0.0)));
    if !q.historical {
        let bug = crate::integrate::red_target::open_bug(q.tx, q.project)?
            .map(|slug| crate::tasks::ops::key_of(q.tx, &slug).map(Evidence::Tasks)).transpose()?;
        out.readings.push(Reading::new("project", q.project, "red_target", f64::from(bug.is_some()),
            bug.into_iter().collect(), Source::Present).fixed(Threshold::Absolute(0.0)));
    }
    Ok(out)
}

struct Verify {
    evidence: Evidence,
    task: String,
    at: i64,
    passed: bool,
    agent: Option<String>,
}

pub(super) fn verify(q: &Query<'_, '_>) -> Result<Measurements, String> {
    if !catalog::integration_on(q.tx, q.project)? { return Ok(Measurements::missing(INTEGRATION_OFF)); }
    let attempts = rows(q.tx, &format!("{} SELECT v.attempt, v.task, v.at, v.passed, v.agent
        FROM integration_verify_results v JOIN tasks t ON t.id = v.task AND t.project = v.project
        WHERE v.project = ?1 AND v.at BETWEEN ?3 AND ?2 AND NOT EXISTS (
            SELECT 1 FROM project_events e WHERE e.task = v.task AND e.kind = 'project' AND e.at >= v.at)
        ORDER BY v.at, v.attempt", project_events()),
        (q.project, q.window.end, q.window.before), |r| Ok(Verify { evidence: Evidence::IntegrationVerify(r.get(0)?),
            task: r.get(1)?, at: r.get(2)?, passed: r.get(3)?, agent: r.get(4)? }))?;
    if attempts.is_empty() { return Ok(Measurements::missing("No own Verify outcomes are retained.")); }
    let mut tasks = BTreeMap::<&str, &Verify>::new();
    let mut agents = BTreeMap::<&str, [Vec<&Verify>; 2]>::new();
    for row in &attempts {
        let current = q.window.current(row.at);
        if current { tasks.insert(&row.task, row); }
        if let Some(agent) = row.agent.as_deref() {
            agents.entry(agent).or_default()[usize::from(current)].push(row);
        }
    }
    let mut out = Measurements {
        readings: tasks.into_iter().map(|(task, row)| Reading::new("task", task, "own_verify_failed",
            f64::from(!row.passed), vec![row.evidence.clone()], Source::Unsized(VERIFY))).collect(),
        complete_metrics: vec!["own_verify_failed"],
        ..Measurements::default()
    };
    for (agent, [old, now]) in agents {
        if [old.len(), now.len()].into_iter().any(|n| n < super::spend::MIN_RATE_SAMPLE) { continue; }
        let rate = |rows: &[&Verify]| rows.iter().filter(|r| !r.passed).count() as f64 / rows.len() as f64;
        out.readings.push(Reading::new("agent", agent, "own_verify_failure_share", rate(&now),
            now.iter().chain(&old).map(|r| r.evidence.clone()).collect(), Source::Unsized(VERIFY)).history(vec![rate(&old)])
            .comparison(Comparison::Above).fixed(Threshold::Change { by: 0.1 }));
    }
    Ok(out)
}

pub(super) fn diffs(q: &Query<'_, '_>) -> Result<Measurements, String> {
    if !catalog::integration_on(q.tx, q.project)? { return Ok(Measurements::missing(INTEGRATION_OFF)); }
    let Some((id, arch)) = probe_rules::architecture(q)? else { return Ok(Measurements::missing(ARCH_MISSING)); };
    let kinds = rows(q.tx, "SELECT id, type FROM tasks WHERE project = ?1 AND type IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM events e WHERE e.task = tasks.id AND e.kind = 'project' AND e.at >= ?2)",
        (q.project, q.window.before), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
        .into_iter().collect::<BTreeMap<_, _>>();
    let mut baseline = BTreeMap::<&str, Vec<f64>>::new();
    for diff in &arch.diffs {
        if (q.window.before..q.window.start).contains(&diff.at)
            && let Some(kind) = kinds.get(&diff.task)
        {
            baseline.entry(kind).or_default().push(diff.lines as f64);
        }
    }
    let mut out = Measurements::default();
    let limits = baseline.into_iter().filter_map(|(kind, mut history)| {
        let limit = match q.detector.threshold {
            Threshold::Baseline { percentile: p, .. } => percentile(&mut history, p),
            _ => history.first().copied(),
        };
        limit.map(|v| (kind, v))
    }).collect::<BTreeMap<_, _>>();
    for diff in arch.diffs {
        if !q.window.current(diff.at) { continue; }
        if let Some(&limit) = kinds.get(&diff.task).and_then(|kind| limits.get(kind.as_str())) {
            out.readings.push(Reading::new("task", &diff.task, "changed_lines", diff.lines as f64,
                vec![Evidence::Tasks(diff.task.clone()), Evidence::SupervisorProbes(id)],
                Source::Unsized("A diff's size is in no weekly project total; the rework a large diff causes is sized by flow.returned.")).history(vec![limit]));
        }
    }
    if out.readings.is_empty() { out.unavailable.push("Diff comparison needs landed tasks of the same type in both windows."); }
    Ok(out)
}
