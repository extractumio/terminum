// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial

use std::collections::{BTreeMap, BTreeSet};

use crate::dispatcher::usage;
use crate::tasks::ops::rows;

use super::flow::{LAUNCH, events, failed_runs, project_events, returned_runs};
use super::impact::{Source, Unit};
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
    Ok(dispositions_since(q, false, q.window.before)?.into_iter().map(|(run, tasks)|
        (run, tasks.into_values().flatten().map(events).collect())).collect())
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
        out.readings.push(Reading::new("task", task, "task_dollars", current.value, current.evidence, Source::RunUsage)
            .history(old.iter().map(|sample| sample.value).collect()).attributed_runs(current.runs));
    }
    let [before, now] = failure_spend;
    if !now.evidence.is_empty() {
        let evidence = now.evidence.into_iter().chain(before.evidence).collect();
        out.readings.push(Reading::new("project", q.project, "failed_run_dollars", now.value, evidence, Source::RunUsage)
            .history(vec![before.value]).attributed_runs(now.runs));
    }
    if missing {
        out.unavailable.push("Some run prices, launches or task-type baselines are unavailable.");
    }
    Ok(out)
}

#[derive(serde::Serialize)]
pub(super) struct Allocation {
    pub rows: Vec<AllocationRow>,
    pub missing: Option<&'static str>,
    pub unpriced_runs: usize,
    #[serde(skip)]
    evidence: Vec<Evidence>,
}

#[derive(serde::Serialize)]
pub(super) struct AllocationRow {
    pub task_type: &'static str,
    pub dollars: f64,
    pub share: Option<f64>,
    pub target: Option<f64>,
}

pub(super) fn allocation_view(tx: &rusqlite::Transaction, project: &str, start: i64, end: i64,
    targets: &BTreeMap<String, f64>) -> Result<Allocation, String>
{
    let assignments = rows(tx, "WITH spent AS (SELECT DISTINCT u.run FROM usage u JOIN runs r ON r.id = u.run
            WHERE r.project = ?1 AND u.at BETWEEN ?2 AND ?3)
        SELECT DISTINCT s.run, coalesce(t.type, 'untyped') FROM spent s
        LEFT JOIN run_tasks m ON m.run = s.run LEFT JOIN tasks t ON t.id = m.task AND t.project = ?1", (project, start, end),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut kinds = BTreeMap::<String, &str>::new();
    for (run, kind) in &assignments {
        kinds.entry(run.clone()).and_modify(|k| *k = "untyped").or_insert(kind);
    }
    let mut buckets = BTreeMap::<&str, f64>::new();
    let mut evidence = Vec::new();
    let mut unpriced = BTreeSet::new();
    for usage in usage_reports(tx, project, start, end)? {
        if let Some(dollars) = usage.dollars {
            let kind = kinds.get(&usage.run).copied().unwrap_or("untyped");
            *buckets.entry(kind).or_default() += dollars;
            evidence.push(Evidence::Usage(usage.id));
        } else {
            unpriced.insert(usage.run);
        }
    }
    let total = buckets.values().sum::<f64>();
    let missing = if !unpriced.is_empty() { Some("Some runs lack prices. The amounts below include priced usage only.") }
        else if total <= 0.0 { Some("No priced task spend is recorded for this window.") }
        else { None };
    let actual = crate::tasks::attrs::TYPES.into_iter().map(|(kind, _)| kind).chain(["untyped"])
        .map(|kind| {
            let dollars = buckets.get(kind).copied().unwrap_or(0.0);
            AllocationRow { task_type: kind, dollars, share: (total > 0.0).then(|| dollars / total),
                target: targets.get(kind).copied() }
        }).collect();
    Ok(Allocation { rows: actual, missing, unpriced_runs: unpriced.len(), evidence })
}

pub(super) fn allocation(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let allocation = allocation_view(q.tx, q.project, q.window.start, q.window.end, &q.detector.allocation)?;
    if let Some(reason) = allocation.missing { return Ok(Measurements::missing(reason)); }
    let mut out = Measurements::default();
    for row in allocation.rows {
        let Some(share) = row.share else { continue; };
        if row.task_type == "untyped" {
            out.readings.push(Reading::new("project", q.project, "untyped_share", share, allocation.evidence.clone(), Source::SpendAbove(0.0)));
        } else if let Some(target) = row.target {
            out.readings.push(Reading::new("task_type", row.task_type, "allocation_share", share,
                allocation.evidence.clone(), Source::SpendAbove(target)).comparison(Comparison::Deviation(target)));
        }
    }
    Ok(out)
}

#[derive(Default)]
struct AgentSample {
    tasks: BTreeSet<String>,
    failed: BTreeSet<String>,
    returned: BTreeSet<String>,
    questions: BTreeSet<String>,
    spend: Sample,
    costs: Vec<f64>,
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

pub(super) const MIN_RATE_SAMPLE: usize = 30;

type Dispositions = BTreeMap<String, BTreeMap<String, Vec<String>>>;

fn dispositions_since(q: &Query<'_, '_>, returned: bool, since: i64) -> Result<Dispositions, String> {
    let (clause, name) = if returned { (returned_runs(), "returned_runs") } else { (failed_runs(), "failed_runs") };
    let events = rows(q.tx, &format!("{}, {clause}
        SELECT run, task, id FROM {name} WHERE at >= ?3 AND run IS NOT NULL", project_events()),
        (q.project, q.window.end, since),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = Dispositions::new();
    for (run, task, event) in events {
        out.entry(run).or_default().entry(task).or_default().push(event);
    }
    Ok(out)
}

fn samples(q: &Query<'_, '_>, typed: bool, integration: bool) -> Result<BTreeMap<(String, String), AgentSample>, String> {
    let questions = questions(q)?;
    let usage = if typed { usage_rows(q)? } else { BTreeMap::new() };
    let failures = if typed { dispositions_since(q, false, q.window.start)? } else { BTreeMap::new() };
    let returned = if typed && integration { dispositions_since(q, true, q.window.start)? } else { BTreeMap::new() };
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
            for (data, found) in [(&failures, &mut sample.failed), (&returned, &mut sample.returned)] {
                if let Some(ids) = data.get(&run.id).and_then(|tasks| tasks.get(task)) {
                    found.insert(run.id.clone());
                    sample.evidence.extend(ids.iter().cloned().map(events));
                }
            }
        }
        for kind in &categories {
            let sample = out.get_mut(&(kind.to_string(), run.agent.clone())).expect("category has a sample");
            sample.runs += 1;
            sample.evidence.push(Evidence::Runs(run.id.clone()));
            if categories.len() == 1 && run.total_tasks == run.tasks.len() as i64 && run.tasks.iter().all(|(_, kind)| kind.is_some())
                && let Some(spend) = usage.get(&run.id).filter(|s| s.complete) {
                sample.spend.add(&run, spend);
                sample.costs.push(spend.dollars);
            }
        }
    }
    Ok(out)
}

/// A rate and the number of observations behind it, so its excess over the band is a count.
type Rate = fn(&AgentSample) -> Option<(f64, f64)>;

fn rate(count: usize, observations: usize) -> Option<(f64, f64)> {
    (observations >= MIN_RATE_SAMPLE).then(|| (count as f64 / observations as f64, observations as f64))
}
fn question_rate(s: &AgentSample) -> Option<(f64, f64)> { rate(s.questions.len(), s.tasks.len()) }
fn fail_rate(s: &AgentSample) -> Option<(f64, f64)> { rate(s.failed.len(), s.runs) }
fn return_rate(s: &AgentSample) -> Option<(f64, f64)> { rate(s.returned.len(), s.runs) }
fn cost_rate(s: &AgentSample) -> Option<(f64, f64)> {
    let priced = s.costs.len();
    (priced >= MIN_RATE_SAMPLE && priced == s.runs).then(|| (s.spend.value / priced as f64, priced as f64))
}

const QUESTIONS: &str = "Questions reach the owner as answers of several kinds and some stay unanswered, so no weekly total holds them; owner.answer_latency and improve.operations size the owner's answering.";

/// A metric's quantity, or none when its excess has no weekly total.
fn bands(q: &Query<'_, '_>, typed: bool, integration: bool, metrics: &[(&'static str, Rate, Option<Unit>)]) -> Result<Measurements, String> {
    let samples = samples(q, typed, integration)?;
    let mut out = Measurements::default();
    let mut missing = samples.is_empty();
    for ((kind, agent), sample) in &samples {
        for (metric, rate, unit) in metrics {
            let Some((value, observations)) = rate(sample) else {
                missing = true;
                continue;
            };
            let peers: Vec<_> = samples.iter().filter(|((k, a), _)| k == kind && a != agent)
                .filter_map(|(_, s)| rate(s).map(|(n, _)| (n, s))).collect();
            if peers.is_empty() {
                missing = true;
                continue;
            }
            let evidence = std::iter::once(sample).chain(peers.iter().map(|(_, s)| *s))
                .flat_map(|s| s.evidence.iter().cloned().chain(s.questions.iter().cloned().map(events)).chain(s.spend.evidence.iter().cloned()))
                .collect();
            let (subject_kind, subject) = if typed { ("agent_task_type", format!("{agent}/{kind}")) } else { ("agent", agent.clone()) };
            let source = unit.map_or(Source::Unsized(QUESTIONS), |unit| Source::RateExcess(unit, observations));
            out.readings.push(Reading::new(subject_kind, subject, metric, value, evidence, source)
                .history(peers.iter().map(|(value, _)| *value).collect()));
        }
    }
    if missing {
        out.unavailable.push("Some agent or peer rates lack 30 observations or priced run data.");
    }
    Ok(out)
}

pub(super) fn ask_rate(q: &Query<'_, '_>) -> Result<Measurements, String> {
    bands(q, false, false, &[("questions_per_task", question_rate, None)])
}

pub(super) fn scorecard(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let mut metrics: Vec<(&'static str, Rate, Option<Unit>)> = vec![("failed_run_share", fail_rate, Some(Unit::FailureEvents)),
        ("questions_per_task", question_rate, None), ("dollars_per_run", cost_rate, Some(Unit::Spend))];
    let integration = super::catalog::integration_on(q.tx, q.project)?;
    if integration { metrics.push(("returned_run_share", return_rate, Some(Unit::RunEvents))); }
    bands(q, true, integration, &metrics)
}

pub(super) fn review_rates(tx: &rusqlite::Transaction, project: &str, window: super::Window) -> Result<Vec<serde_json::Value>, String> {
    use serde_json::json;
    use super::rate_intervals as rate;
    let detector = super::Detector::for_rule(super::Rule::AgentScorecard);
    let q = Query { tx, project, detector: &detector, window, historical: false };
    let integration = super::catalog::integration_on(tx, project)?;
    let mut observed = samples(&q, true, integration)?;
    let mut rates = Vec::new();
    for agent in crate::tasks::agents::of_project(tx, project)? {
        for (kind, _) in crate::tasks::attrs::TYPES {
            let s = observed.remove(&(kind.to_string(), agent.id.clone())).unwrap_or_default();
            rates.push(json!({"task_type":kind,"agent":agent.id,"agent_name":agent.name,
                "failure":rate::share(s.failed.len(), s.runs, None),
                "return":rate::share(s.returned.len(), s.runs, (!integration).then_some("Project integration is off.")),
                "questions":rate::questions(s.questions.len(), s.tasks.len()),
                "cost":rate::cost(&s.costs, s.runs)}));
        }
    }
    Ok(rates)
}
