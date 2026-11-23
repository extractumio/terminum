// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Retained measurements and the rules that depend on them (AC-218).

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction};

use crate::app::Shared;
use crate::dispatcher::{runs::STILL, waits};
use crate::tasks::ops::{rows, sql};

use super::{Comparison, DAY, Evidence, Measurements, Query, Reading, Rule, impact::Source, percentile};

pub(super) const CADENCE: i64 = 15 * 60;
const QUEUE: &str = "queue_waits";
const QUEUE_AGE: &str = "queue_oldest_minutes";
const QUEUE_BASELINE: &str = "queue_waits_p90";
const LIMIT: &str = "subscription_used";
const ETA: &str = "plan_eta";

pub(super) struct Point {
    metric: &'static str,
    subject: String,
    value: f64,
    resets_at: Option<i64>,
}

impl Point {
    fn new(metric: &'static str, subject: impl Into<String>, value: f64) -> Self {
        Self { metric, subject: subject.into(), value, resets_at: None }
    }
}

pub(super) fn measure(
    tx: &Transaction, app: &Shared, project: &str, at: i64, daily: bool,
) -> Result<Vec<Point>, String> {
    let (count, oldest) = waits::queue_depth(tx, project)?;
    let mut samples = vec![Point::new(QUEUE, "", count as f64),
        Point::new(QUEUE_AGE, "", oldest.map_or(0.0, |old| (at - old).max(0) as f64 / 60.0))];
    if daily {
        let mut history = rows(tx, "SELECT value FROM supervisor_points
            WHERE project = ?1 AND metric = ?2 AND subject = '' AND at >= ?3",
            (project, QUEUE, at - 28 * DAY), |r| r.get(0))?;
        history.push(count as f64);
        if let Some(baseline) = percentile(&mut history, 90) {
            samples.push(Point::new(QUEUE_BASELINE, "", baseline));
        }
    }

    let kinds = rows(tx, &format!("SELECT DISTINCT kind FROM agents WHERE project = ?1
        UNION SELECT DISTINCT kind FROM runs r WHERE project = ?1 AND {STILL}"), [project], |r| r.get::<_, String>(0))?;
    for kind in kinds {
        let Some((read, windows)) = app.dispatcher.subscriptions.latest(&kind) else { continue };
        if !(0..=3600).contains(&(at - read)) {
            continue;
        }
        let by_length: BTreeMap<_, _> = windows.into_iter().map(|window| (window.minutes, window)).collect();
        samples.extend(by_length.into_values().map(|w| Point { metric: LIMIT, subject: format!("{kind}/{}", w.minutes),
            value: w.used, resets_at: w.resets_at }));
    }
    let panes = rows(tx, &format!("SELECT r.pane_key FROM runs r WHERE r.project = ?1 AND {STILL} AND r.pane_key IS NOT NULL
        UNION SELECT m.pane_key FROM project_members m WHERE m.project = ?1
        AND NOT EXISTS (SELECT 1 FROM runs r WHERE r.pane_key = m.pane_key AND {STILL})"),
        [project], |r| r.get::<_, String>(0))?;
    let mut etas = BTreeMap::new();
    for pane in panes {
        if let Some((session, eta, updated)) = app.plans.supervisor_eta(&pane) {
            let latest = etas.entry(session).or_insert((updated, eta));
            *latest = (*latest).max((updated, eta));
        }
    }
    samples.extend(etas.into_iter().map(|(session, (_, eta))| Point::new(ETA, session, eta as f64)));
    Ok(samples)
}

pub(super) fn write(tx: &Transaction, project: &str, at: i64, samples: &[Point]) -> Result<(), String> {
    if samples.is_empty() {
        return Ok(());
    }
    let mut put = tx.prepare_cached("INSERT INTO supervisor_points (project, metric, subject, at, value, resets_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6)").map_err(sql)?;
    for point in samples {
        put.execute((project, point.metric, &point.subject, at, point.value, point.resets_at)).map_err(sql)?;
    }
    Ok(())
}

struct Sample {
    id: i64,
    at: i64,
    value: f64,
    reset: Option<i64>,
}

/// A snapshot rule's metric, its subjects and why it sizes no impact share.
fn snapshot_metric(rule: Rule) -> (&'static str, &'static str, &'static str) {
    match rule {
        Rule::QueueGrowth => (QUEUE, "project",
            "Dispatcher queue samples are a state; queued tasks' delay is sized by flow.starved and flow.wait_human_age."),
        Rule::LimitNear => (LIMIT, "subscription",
            "A predicted subscription limit is a forecast, not spent quantity; spend is sized by flow.cost_outlier."),
        Rule::StalePlan => (ETA, "session",
            "A growing plan estimate is a state; the delay it causes shows in lead time, which flow.lead_time_shift sizes."),
        _ => unreachable!("Only snapshot rules use the snapshot query"),
    }
}

pub(super) fn first_point(tx: &Transaction, project: &str, rule: Rule, at: i64) -> Result<Option<i64>, String> {
    tx.query_row("SELECT min(at) FROM supervisor_points WHERE project = ?1 AND metric = ?2 AND at <= ?3",
        (project, snapshot_metric(rule).0, at), |r| r.get(0)).map_err(sql)
}

pub(super) fn snapshots(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let (metric, subjects, no_share) = snapshot_metric(q.detector.rule);
    let mut groups = BTreeMap::<String, Vec<Sample>>::new();
    let samples = rows(q.tx, "SELECT id, subject, at, value, resets_at FROM supervisor_points
        WHERE project = ?1 AND metric = ?2 AND at BETWEEN ?3 AND ?4 ORDER BY subject, at DESC",
        (q.project, metric, q.window.end - 4 * CADENCE, q.window.end),
        |r| Ok((r.get::<_, String>(1)?, Sample { id: r.get(0)?, at: r.get(2)?, value: r.get(3)?, reset: r.get(4)? })))?;
    for (subject, point) in samples {
        groups.entry(subject).or_default().push(point);
    }
    let baseline = if metric == QUEUE {
        q.tx.query_row("SELECT value FROM supervisor_points WHERE project = ?1 AND metric = ?2
            AND subject = '' AND at <= ?3 ORDER BY at DESC LIMIT 1", (q.project, QUEUE_BASELINE, q.window.end),
            |r| r.get::<_, f64>(0)).optional().map_err(sql)?
    } else {
        None
    };
    let mut out = Measurements::missing("Not enough retained snapshot points for this period.");
    for (subject, points) in groups {
        let (value, used) = match (metric, points.as_slice()) {
            (LIMIT, [a, b, ..]) if a.reset == b.reset && a.reset.is_some_and(|reset| reset > q.window.end)
                && a.at - b.at <= 2 * CADENCE => {
                let seconds = a.reset.unwrap_or(a.at) - a.at;
                let projected = a.value + (a.value - b.value).max(0.0) * seconds as f64 / (a.at - b.at) as f64;
                (projected, 2)
            }
            (QUEUE | ETA, [a, b, c, ..]) if a.at - c.at <= 3 * CADENCE => {
                let expected = baseline.unwrap_or(c.value).max(c.value);
                let rise = if a.value > b.value && b.value > c.value { a.value - expected } else { 0.0 };
                let scale = if metric == ETA { 60.0 } else { 1.0 };
                (rise / scale, 3)
            }
            _ => continue,
        };
        if q.window.end - points[0].at >= 2 * CADENCE {
            continue;
        }
        let id = if subject.is_empty() { q.project.to_owned() } else { subject };
        let evidence = points[..used].iter().map(|p| Evidence::SupervisorPoints(p.id)).collect();
        let reading = Reading::new(subjects, id, metric, value, evidence, Source::Unsized(no_share));
        out.readings.push(if metric == LIMIT {
            reading.comparison(Comparison::AtLeast)
        } else { reading });
    }
    if !out.readings.is_empty() {
        out.unavailable.clear();
    }
    Ok(out)
}

fn counter_events(rule: Rule) -> &'static str {
    match rule {
        Rule::Loop => "('failure', 'repeat', 'compaction')",
        Rule::Subagents => "('subagent_start')",
        _ => unreachable!("Only hook rules use the hook counter query"),
    }
}

pub(super) fn first_count(tx: &Transaction, project: &str, rule: Rule, at: i64) -> Result<Option<i64>, String> {
    let events = counter_events(rule);
    tx.query_row(&format!("SELECT min(at) FROM supervisor_hook_counts WHERE project = ?1 AND at <= ?2 AND event IN {events}"),
        (project, at), |r| r.get(0)).map_err(sql)
}

pub(super) fn counters(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let filters = counter_events(q.detector.rule);
    let start = q.window.end - DAY + 1;
    let counts = rows(q.tx, &format!("SELECT agent, session, event, sum(count), max(id)
        FROM supervisor_hook_counts WHERE project = ?1 AND at BETWEEN ?2 AND ?3
            AND event IN {filters} GROUP BY agent, session, event"), (q.project, start, q.window.end),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, f64>(3)?, r.get::<_, i64>(4)?)))?;
    let mut histories = BTreeMap::<String, Vec<f64>>::new();
    for (event, count) in rows(q.tx, &format!("SELECT event, sum(count) FROM supervisor_hook_counts
        WHERE project = ?1 AND at >= ?2 AND at < ?3 AND event IN {filters}
        GROUP BY agent, session, event, at / {DAY}"), (q.project, q.window.start, start),
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, f64>(1)?)))? {
        histories.entry(event).or_default().push(count);
    }
    let mut out = Measurements::default();
    for (agent, session, event, count, id) in counts {
        let metric = match event.as_str() {
            "failure" => "tool_failures",
            "repeat" => "repeated_tools",
            "compaction" => "compactions",
            _ => "subagent_starts",
        };
        out.readings.push(Reading::new("session", format!("{agent}/{session}"), metric, count,
            vec![Evidence::SupervisorHookCounts(id)],
            Source::Unsized("A session's tool failures, repeats, compactions and subagents are in no weekly total, and its whole spend would overstate their cost.")).history(histories.get(&event).cloned().unwrap_or_default()));
    }
    let available = q.tx.query_row("SELECT EXISTS (SELECT 1 FROM supervisor_hook_counts
        WHERE project = ?1 AND at <= ?2)", (q.project, q.window.end), |r| r.get::<_, bool>(0)).map_err(sql)?;
    if !available {
        out.unavailable.push("No retained hook counters for this period.");
    }
    Ok(out)
}
