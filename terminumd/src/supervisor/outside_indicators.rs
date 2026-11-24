// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-262: guard optional probe series without putting their verdicts into the seven.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use crate::tasks::ops::rows;

use super::{Comparison, Evidence, Measurements, Query, Threshold, WEEK, impact::{Source, absolute_noise}, probes,
    scoreboard::{self, Verdict}};

const LEGACY: &str = "Choose a metric and a guard in project settings.";
const UNSIZED: Source = Source::Unsized("An outside product metric is in no weekly task, owner or spend total.");

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum Setting {
    Guarded(Guarded),
    Legacy(String),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Guarded {
    probe: String,
    metric: String,
    higher_is_better: bool,
    guard: String,
    guard_higher_is_better: bool,
}

pub(super) fn validate(settings: &BTreeMap<String, Setting>) -> Result<(), String> {
    if settings.len() > 32 { return Err("Set at most 32 outside indicators.".into()); }
    for (name, setting) in settings {
        // Older packs supplied a name and probe ID without a metric or guard.
        let Setting::Guarded(config) = setting else { continue; };
        if name.trim().is_empty() || name.len() > 100 || name.chars().any(crate::termtext::control) ||
            !probes::probe_id(&config.probe) || !probes::identifier(&config.metric, 80) ||
            !probes::identifier(&config.guard, 80) || config.guard == config.metric
        {
            return Err("An outside indicator needs a name, a probe ID and distinct metric and guard IDs.".into());
        }
    }
    Ok(())
}

#[derive(Serialize)]
pub(super) struct Indicator {
    name: String,
    probe: String,
    verdict: &'static str,
    guard: String,
    metrics: Vec<Metric>,
}

#[derive(Serialize)]
struct Metric {
    name: String,
    unit: Option<String>,
    verdict: &'static str,
    before: Option<f64>,
    current: Option<f64>,
    noise: f64,
    missing: Option<&'static str>,
    #[serde(skip)]
    samples: Vec<i64>,
}

struct Reading {
    sample: i64,
    week: usize,
    value: f64,
    unit: String,
}

fn readings(tx: &Transaction, project: &str, probe: &str, config: &str, at: i64, wanted: &BTreeSet<&str>)
    -> Result<BTreeMap<String, Vec<Reading>>, String>
{
    let start = at - scoreboard::WEEKS as i64 * WEEK;
    let points = rows(tx, "WITH inputs AS (
        SELECT s.id, json_extract(m.value, '$.at') AS time,
               json_extract(m.value, '$.metric') AS metric,
               json_extract(m.value, '$.value') AS value, json_extract(m.value, '$.unit') AS unit
        FROM supervisor_probe_samples s, json_each(s.data) m
        WHERE s.project = ?1 AND s.source = ?2 AND s.config = ?3 AND s.at > ?4 AND s.at <= ?5
          AND s.state = 'measured' AND json_extract(m.value, '$.dims') = '{}'
    ), weekly AS (
        SELECT *, (time - ?4 - 1) / ?6 AS week FROM inputs
        WHERE time > ?4 AND time <= ?5 AND metric IN (SELECT value FROM json_each(?7))
    ), latest AS (
        SELECT *, row_number() OVER (PARTITION BY metric, week ORDER BY time DESC, id DESC) AS position
        FROM weekly
    )
    SELECT metric, week, value, unit, id FROM latest WHERE position = 1",
        (project, format!("custom:{probe}"), config, start, at, WEEK, serde_json::json!(wanted).to_string()),
        |r| Ok((r.get::<_, String>(0)?, Reading { week: usize::from(r.get::<_, u8>(1)?), value: r.get(2)?, unit: r.get(3)?, sample: r.get(4)? })))?;
    let mut by_metric = BTreeMap::<_, Vec<_>>::new();
    for (name, point) in points { by_metric.entry(name).or_default().push(point); }
    Ok(by_metric)
}

fn metric(name: &str, readings: &[Reading], higher_is_better: bool, missing: &'static str)
    -> (Metric, Verdict)
{
    let mut series = [None; scoreboard::WEEKS];
    let unit = readings.first().map(|row| &row.unit);
    let units_match = readings.iter().all(|row| Some(&row.unit) == unit);
    if units_match {
        for row in readings { series[row.week] = Some(row.value); }
    }
    let average = |weeks: &[Option<f64>]| {
        weeks.iter().try_fold(0.0, |sum, value| Some(sum + (*value)? / weeks.len() as f64))
    };
    let before = average(&series[..4]);
    let current = average(&series[4..]);
    let noise = absolute_noise(&series).min(f64::MAX);
    let verdict = scoreboard::compare(before, current, noise, Some(higher_is_better));
    let missing = if before.zip(current).is_some() { None }
        else if units_match { Some(missing) }
        else { Some("Probe units changed between readings.") };
    let samples = readings.iter().map(|row| row.sample).collect();
    (Metric { name: name.into(), unit: unit.filter(|_| units_match).cloned(), verdict: verdict.label(), before, current, noise, missing, samples }, verdict)
}

pub(super) fn read(tx: &Transaction, project: &str, at: i64, settings: &BTreeMap<String, Setting>)
    -> Result<Vec<Indicator>, String>
{
    if settings.is_empty() { return Ok(Vec::new()); }
    let configs = probes::custom_configs(tx, project)?;
    let mut wanted = BTreeMap::<&str, BTreeSet<&str>>::new();
    for setting in settings.values() {
        if let Setting::Guarded(setting) = setting {
            wanted.entry(&setting.probe).or_default().extend([setting.metric.as_str(), &setting.guard]);
        }
    }
    let mut sources = BTreeMap::new();
    for (probe, wanted) in wanted {
        if let Some(config) = configs.get(probe) {
            sources.insert(probe, readings(tx, project, probe, config, at, &wanted)?);
        }
    }
    settings.iter().map(|(name, setting)| {
        let setting = match setting {
            Setting::Guarded(setting) => setting,
            Setting::Legacy(probe) => return Ok(Indicator { name: name.clone(), probe: probe.clone(),
                verdict: Verdict::Unmeasured.label(), guard: LEGACY.into(),
                metrics: Vec::new() }),
        };
        let source = sources.get(setting.probe.as_str());
        let values = |name: &str| source.and_then(|metrics| metrics.get(name)).map_or(&[][..], Vec::as_slice);
        let missing = if source.is_some() { "The probe needs a reading in each week of the comparison." }
            else { "The custom probe is not approved." };
        let (main, verdict) = metric(&setting.metric, values(&setting.metric), setting.higher_is_better, missing);
        let (guard, guard_verdict) = metric(&setting.guard, values(&setting.guard), setting.guard_higher_is_better, missing);
        Ok(Indicator { name: name.clone(), probe: setting.probe.clone(), guard: format!("Guard metric · {}", setting.guard),
            verdict: scoreboard::guarded(verdict, guard_verdict).label(), metrics: vec![main, guard] })
    }).collect()
}

/// `product.external`: each guarded outside indicator's metric whose four-week average moved against its direction
/// by more than its weekly noise plus the detector's margin, over the scoreboard's two four-week windows.
pub(super) fn external(q: &Query<'_, '_>) -> Result<Measurements, String> {
    let settings = super::settings::read(q.tx, q.project)?.settings.charter.outside_indicators;
    if settings.is_empty() { return Ok(Measurements::missing("No outside indicator is set in the charter.")); }
    let margin = match q.detector.threshold { Threshold::Change { by } => by, _ => 0.0 };
    let mut out = Measurements::default();
    for (setting, indicator) in settings.values().zip(read(q.tx, q.project, q.window.end, &settings)?) {
        let (Setting::Guarded(setting), Some(main)) = (setting, indicator.metrics.into_iter().next()) else {
            out.unavailable.push(LEGACY);
            continue;
        };
        let Some((before, current)) = main.before.zip(main.current) else {
            out.unavailable.extend(main.missing);
            continue;
        };
        let worse = if setting.higher_is_better { Comparison::Below } else { Comparison::Above };
        let evidence = main.samples.into_iter().map(Evidence::SupervisorProbes).collect();
        out.readings.push(super::Reading::new("outside_indicator", indicator.name, "outside_metric_average", current, evidence, UNSIZED)
            .history(vec![before]).comparison(worse).fixed(Threshold::Change { by: main.noise + margin }));
    }
    Ok(out)
}
