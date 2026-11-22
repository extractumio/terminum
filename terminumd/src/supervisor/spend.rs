// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use std::collections::{BTreeMap, BTreeSet};

use crate::dispatcher::usage;
use crate::tasks::ops::rows;

use super::flow::{LAUNCH, events, failed_runs, project_events};
use super::impact::Source;
use super::{Comparison, Evidence, Measurements, Query, Reading};

/// Read only the numeric reports and stable row IDs. No usage report's text enters the signal.
#[derive(Default)]
struct Spend {
    dollars: f64,
    complete: bool,
    evidence: Vec<Evidence>,
}

pub(super) struct UsageReport {
    pub at: i64,
    pub run: String,
    pub id: i64,
    pub tokens: f64,
    pub dollars: Option<f64>,
}

pub(super) fn usage_reports(tx: &rusqlite::Transaction, project: &str, start: i64, end: i64) -> Result<Vec<UsageReport>, String> {
    rows(tx, &format!("SELECT u.at, u.run, u.rowid, u.model, {} FROM usage u JOIN runs r ON r.id = u.run
        WHERE r.project = ?1 AND u.at BETWEEN ?2 AND ?3 ORDER BY u.at, u.rowid", usage::TOTALS),
        (project, start, end), |r| {
            let tokens = usage::totals_at(r, 4)?;
            let model: Option<String> = r.get(3)?;
            Ok(UsageReport { at: r.get(0)?, run: r.get(1)?, id: r.get(2)?,
                tokens: (tokens.input + tokens.output + tokens.cache_write + tokens.cache_read) as f64,
                dollars: model.as_deref().and_then(|model| usage::dollars(&tokens, model)) })
        })
}

fn usage_rows(q: &Query<'_, '_>) -> Result<BTreeMap<String, Spend>, String> {
    let mut out = BTreeMap::new();
    for row in usage_reports(q.tx, q.project, q.window.before, q.window.end)? {
        let spend = out.entry(row.run).or_insert_with(|| Spend { complete: true, ..Spend::default() });
        if let Some(dollars) = row.dollars {
            spend.dollars += dollars;
        } else {
            spend.complete = false;
        }
        spend.evidence.push(Evidence::Usage(row.id));
    }
    Ok(out)
}

struct Run {
    id: String,
    agent: String,
    at: i64,
    tasks: Vec<(String, Option<String>)>,
    total_tasks: i64,
}

fn runs(q: &Query<'_, '_>) -> Result<Vec<Run>, String> {
    let rows = rows(q.tx, &format!("SELECT r.id, r.agent_id, launch.at, t.id, t.type, (SELECT count(*) FROM run_tasks a WHERE a.run = r.id) FROM runs r
        JOIN run_tasks m ON m.run = r.id JOIN tasks t ON t.id = m.task
        JOIN inputs launch ON launch.id = (SELECT i.id FROM inputs i WHERE i.run = r.id
            AND {LAUNCH}
            ORDER BY i.at, i.id LIMIT 1)
        WHERE r.project = ?1 AND t.project = ?1 AND launch.at BETWEEN ?2 * 1000 AND ?3 * 1000 + 999
        ORDER BY r.id, t.id"), (q.project, q.window.before, q.window.end), |r| Ok((
        r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? / 1000,
        r.get::<_, String>(3)?, r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)?)))?;
    let mut out: BTreeMap<String, Run> = BTreeMap::new();
    for (id, agent, at, task, kind, total_tasks) in rows {
        out.entry(id.clone()).or_insert_with(|| Run { id, agent, at, tasks: vec![], total_tasks }).tasks.push((task, kind));
    }
    Ok(out.into_values().collect())
}

fn failures(q: &Query<'_, '_>) -> Result<BTreeMap<String, Vec<Evidence>>, String> {
    let events_sql = project_events();
    let failed_sql = failed_runs();
    let failed = rows(q.tx, &format!("{events_sql}, {failed_sql}
        SELECT run, id FROM failed_runs WHERE at >= ?3 AND run IS NOT NULL ORDER BY run, at, id"),
        (q.project, q.window.end, q.window.before), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = BTreeMap::<String, Vec<Evidence>>::new();
    for (run, event) in failed {
        out.entry(run).or_default().push(events(event));
    }
    Ok(out)
}

#[derive(Default)]
struct Sample {
    value: f64,
    evidence: Vec<Evidence>,
    runs: BTreeSet<String>,
}

impl Sample {
    fn add(&mut self, run: &Run, spend: &Spend) {
        self.value += spend.dollars;
        self.runs.insert(run.id.clone());
        self.evidence.push(Evidence::Runs(run.id.clone()));
        self.evidence.extend(spend.evidence.iter().cloned());
    }
}

pub(super) fn outliers(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let usage = usage_rows(q)?;
    let failed = failures(q)?;
    let mut old_tasks = BTreeMap::<(&str, &str), Sample>::new();
    let mut current = BTreeMap::<(&str, &str), Sample>::new();
    let mut failure_spend: [Sample; 2] = Default::default();
    let runs = runs(q)?;
    let mut missing = usage.is_empty() || runs.is_empty();
    for run in &runs {
        let Some(spend) = usage.get(&run.id).filter(|s| s.complete) else {
            missing = true;
            continue;
        };
        let is_current = q.window.current(run.at);
        if let Some(evidence) = failed.get(&run.id) {
            let sample = &mut failure_spend[usize::from(is_current)];
            sample.add(run, spend);
            sample.evidence.extend(evidence.iter().cloned());
        }
        // A run carrying several tasks has no reliable split for a per-task outlier.
        if run.total_tasks == 1 && let [(task, Some(kind))] = run.tasks.as_slice() {
            if is_current {
                current.entry((task, kind)).or_default().add(run, spend);
            } else {
                old_tasks.entry((task, kind)).or_default().add(run, spend);
            }
        }
    }
    let mut old = BTreeMap::<&str, Vec<Sample>>::new();
    for ((_, kind), sample) in old_tasks {
        old.entry(kind).or_default().push(sample);
    }
    let mut out = Measurements::default();
    for ((task, kind), mut current) in current {
        let Some(old) = old.get(kind) else {
            missing = true;
            continue;
        };
        current.evidence.extend(old.iter().flat_map(|sample| sample.evidence.iter().cloned()));
        current.evidence.push(Evidence::Tasks(task.into()));
        out.readings.push(Reading::new("task", task, "task_dollars", current.value, current.evidence).source(Source::RunUsage)
            .history(old.iter().map(|sample| sample.value).collect()).attributed_runs(current.runs));
    }
    let [before, now] = failure_spend;
    if !now.evidence.is_empty() {
        let evidence = now.evidence.into_iter().chain(before.evidence).collect();
        out.readings.push(Reading::new("project", q.project, "failed_run_dollars", now.value, evidence).source(Source::RunUsage)
            .history(vec![before.value]).attributed_runs(now.runs));
    }
    if missing {
        out.unavailable.push("Some run prices, launches or task-type baselines are unavailable.");
    }
    Ok(out)
}

pub(super) fn allocation(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let usage = usage_rows(q)?;
    let mut buckets = BTreeMap::<Option<&str>, f64>::new();
    let mut measured = vec![];
    let mut total = 0.0;
    let mut missing = false;
    let runs = runs(q)?;
    for run in runs.iter().filter(|run| q.window.current(run.at)) {
        let Some(spend) = usage.get(&run.id).filter(|s| s.complete) else {
            missing = true;
            continue;
        };
        let kind = &run.tasks[0].1;
        let consistent = run.total_tasks == run.tasks.len() as i64 && run.tasks.iter().all(|(_, k)| k == kind);
        *buckets.entry(if consistent { kind.as_deref() } else { None }).or_default() += spend.dollars;
        total += spend.dollars;
        measured.push(Evidence::Runs(run.id.clone()));
        measured.extend(spend.evidence.iter().cloned());
    }
    if missing || usage.is_empty() || total == 0.0 {
        return Ok(Measurements::missing("Allocation needs priced Dispatcher spend in the window."));
    }
    let mut out = Measurements::default();
    if let Some(untyped) = buckets.get(&None) {
        out.readings.push(Reading::new("project", q.project, "untyped_share", untyped / total, measured.clone()).source(Source::UnclassifiedSpend));
    }
    if q.detector.allocation.is_empty() {
        out.unavailable.push("No project allocation targets are configured.");
        out.complete_metrics.push("untyped_share");
    } else {
        for (kind, target) in &q.detector.allocation {
            let share = buckets.get(&Some(kind.as_str())).copied().unwrap_or(0.0) / total;
            out.readings.push(Reading::new("task_type", kind, "allocation_share", share,
                measured.clone()).comparison(Comparison::Deviation(*target)));
        }
    }
    Ok(out)
}

#[derive(Default)]
struct AgentSample {
    tasks: BTreeSet<String>,
    failed: BTreeSet<String>,
    questions: BTreeSet<String>,
    spend: Sample,
    costs: usize,
    runs: usize,
    evidence: Vec<Evidence>,
}

fn questions(q: &Query<'_, '_>) -> Result<BTreeMap<(String, String), Vec<String>>, String> {
    let events_sql = project_events();
    let questions = rows(q.tx, &format!("{events_sql}
        SELECT e.task, substr(e.author, length('agent:pool:') + 1), e.id FROM project_events e
        WHERE e.kind = 'comment' AND json_extract(e.data, '$.questions') IS NOT NULL
        AND e.author GLOB 'agent:pool:*' AND e.at >= ?3"),
        (q.project, q.window.end, q.window.start), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = BTreeMap::<_, Vec<_>>::new();
    for (task, agent, id) in questions {
        out.entry((task, agent)).or_default().push(id);
    }
    Ok(out)
}

const MIN_RATE_SAMPLE: usize = 30;

fn samples(q: &Query<'_, '_>, typed: bool) -> Result<BTreeMap<(String, String), AgentSample>, String> {
    let questions = questions(q)?;
    let usage = if typed { usage_rows(q)? } else { BTreeMap::new() };
    let failures = if typed { failures(q)? } else { BTreeMap::new() };
    let mut out = BTreeMap::<_, AgentSample>::new();
    for run in runs(q)?.into_iter().filter(|run| q.window.current(run.at)) {
        let mut categories = BTreeSet::new();
        for (task, kind) in &run.tasks {
            let kind = if typed {
                let Some(kind) = kind else { continue };
                kind.as_str()
            } else {
                ""
            };
            categories.insert(kind);
            let sample = out.entry((kind.to_string(), run.agent.clone())).or_default();
            if sample.tasks.insert(task.clone()) {
                sample.evidence.push(Evidence::Tasks(task.clone()));
                if let Some(found) = questions.get(&(task.clone(), run.agent.clone())) {
                    sample.questions.extend(found.iter().cloned());
                }
            }
            if let Some(failure) = failures.get(&run.id) {
                sample.failed.insert(run.id.clone());
                sample.evidence.extend(failure.iter().cloned());
            }
        }
        for kind in &categories {
            let sample = out.get_mut(&(kind.to_string(), run.agent.clone())).expect("category has a sample");
            sample.runs += 1;
            sample.evidence.push(Evidence::Runs(run.id.clone()));
            if categories.len() == 1 && run.total_tasks == run.tasks.len() as i64 && run.tasks.iter().all(|(_, kind)| kind.is_some())
                && let Some(spend) = usage.get(&run.id).filter(|s| s.complete) {
                sample.spend.add(&run, spend);
                sample.costs += 1;
            }
        }
    }
    Ok(out)
}

type Rate = fn(&AgentSample) -> Option<f64>;

fn question_rate(s: &AgentSample) -> Option<f64> {
    (s.tasks.len() >= MIN_RATE_SAMPLE).then(|| s.questions.len() as f64 / s.tasks.len() as f64)
}
fn fail_rate(s: &AgentSample) -> Option<f64> {
    (s.runs >= MIN_RATE_SAMPLE).then(|| s.failed.len() as f64 / s.runs as f64)
}
fn cost_rate(s: &AgentSample) -> Option<f64> {
    (s.costs >= MIN_RATE_SAMPLE && s.costs == s.runs).then(|| s.spend.value / s.costs as f64)
}

fn bands(q: &Query<'_, '_>, typed: bool, metrics: &[(&'static str, Rate)]) -> Result<Measurements, String> {
    let samples = samples(q, typed)?;
    let mut out = Measurements::default();
    let mut missing = samples.is_empty();
    for ((kind, agent), sample) in &samples {
        for (metric, rate) in metrics {
            let Some(value) = rate(sample) else {
                missing = true;
                continue;
            };
            let peers: Vec<_> = samples.iter().filter(|((k, a), _)| k == kind && a != agent)
                .filter_map(|(_, s)| rate(s).map(|n| (n, s))).collect();
            if peers.is_empty() {
                missing = true;
                continue;
            }
            let evidence = std::iter::once(sample).chain(peers.iter().map(|(_, s)| *s))
                .flat_map(|s| s.evidence.iter().cloned().chain(s.questions.iter().cloned().map(events)).chain(s.spend.evidence.iter().cloned()))
                .collect();
            let (subject_kind, subject) = if typed { ("agent_task_type", format!("{agent}/{kind}")) } else { ("agent", agent.clone()) };
            out.readings.push(Reading::new(subject_kind, subject, metric, value, evidence)
                .history(peers.iter().map(|(value, _)| *value).collect()));
        }
    }
    if missing {
        out.unavailable.push("Some agent or peer rates lack 30 observations or priced run data.");
    }
    Ok(out)
}

pub(super) fn ask_rate(q: &Query<'_, '_>) -> Result<Measurements, String> {
    bands(q, false, &[("questions_per_task", question_rate)])
}

pub(super) fn scorecard(q: &Query<'_, '_>) -> Result<Measurements, String> {
    bands(q, true, &[("failed_run_share", fail_rate), ("questions_per_task", question_rate), ("dollars_per_run", cost_rate)])
}
