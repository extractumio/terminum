// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Deterministic Supervisor queries (AC-211, AC-212). Evaluation is read-only; recording is explicit.

mod catalog;
mod flow;
mod people;
mod signals;
mod spend;

#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod catalog_tests;

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::tasks::{ops::check_project, store::Store};

pub(crate) use catalog::Rule;
pub(crate) use signals::Signal;

const SCHEMA: usize = 41;
const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Mode {
    Off,
    Shadow,
    Live,
    Digest,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Threshold {
    Absolute(f64),
    Baseline { percentile: u8, floor: f64 },
    Change { by: f64 },
}

impl Threshold {
    fn valid(self) -> bool {
        let nonnegative = |n: f64| n.is_finite() && n >= 0.0;
        match self {
            Self::Absolute(limit) => nonnegative(limit),
            Self::Baseline { percentile, floor } => (1..=100).contains(&percentile) && nonnegative(floor),
            Self::Change { by } => nonnegative(by),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Detector {
    pub rule: Rule,
    pub mode: Mode,
    pub window_days: u16,
    pub threshold: Threshold,
    pub allocation: BTreeMap<String, f64>,
    pub scores: Scores,
}

impl Detector {
    pub(crate) fn named(name: &str) -> Result<Self, String> {
        let rule = Rule::named(name)?;
        let threshold = rule.defaults();
        let window_days = if rule == Rule::FailedWithoutReason { 7 } else { 28 };
        Ok(Self { rule, mode: Mode::Off, window_days, threshold, allocation: BTreeMap::new(), scores: Scores::default() })
    }

    /// Initial attachment can set its mode in the pack. Enabling an additional rule always starts here.
    pub(crate) fn enable(&mut self) {
        if self.mode == Mode::Off {
            self.mode = Mode::Shadow;
        }
    }

    fn valid(&self) -> bool {
        (1..=365).contains(&self.window_days) && self.threshold.valid() && self.scores.valid()
            && self.rule.supports(self.threshold)
            && self.allocation.keys().all(|k| crate::tasks::attrs::TYPES.iter().any(|(kind, _)| kind == k))
            && self.allocation.values().all(|share| share.is_finite() && (0.0..=1.0).contains(share))
            && (self.allocation.is_empty() || (self.allocation.values().sum::<f64>() - 1.0).abs() < 1e-9)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Scores {
    pub impact: u8,
    pub blast: u8,
    pub reversibility: u8,
    pub confidence: f64,
}

impl Default for Scores {
    fn default() -> Self {
        Self { impact: 0, blast: 0, reversibility: 0, confidence: 1.0 }
    }
}

impl Scores {
    fn valid(self) -> bool {
        self.level() <= 3 && (0.0..=1.0).contains(&self.confidence)
    }

    pub(crate) fn level(self) -> u8 {
        self.impact.max(self.blast).max(self.reversibility)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "table", content = "id", rename_all = "snake_case")]
pub(crate) enum Evidence {
    Tasks(String),
    Events(String),
    Runs(String),
    ScheduleFires(i64),
    TaskLinks { task: String, other: String },
    Inputs(i64),
    Usage(i64),
    UserInteraction(i64),
    Messages(i64),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Measure {
    pub value: f64,
    pub baseline: f64,
    pub threshold: f64,
    pub window_start: i64,
    pub window_end: i64,
}

#[derive(Clone, Debug)]
pub(crate) struct Finding {
    subject_kind: &'static str,
    subject_id: String,
    metric: &'static str,
    measure: Measure,
    evidence: Vec<Evidence>,
}

#[derive(Debug)]
pub(crate) struct Evaluation {
    project: String,
    at: i64,
    pub(crate) detectors: Vec<Checked>,
}

#[derive(Debug)]
pub(crate) struct Checked {
    pub rule: Rule,
    pub mode: Mode,
    pub scores: Scores,
    pub unavailable: Vec<&'static str>,
    pub findings: Vec<Finding>,
    complete_metrics: Vec<&'static str>,
}

/// A detector returns measured subjects; an unavailable input cannot prove anything resolved.
#[derive(Default)]
struct Measurements {
    readings: Vec<Reading>,
    unavailable: Vec<&'static str>,
    complete_metrics: Vec<&'static str>,
}

impl Measurements {
    fn missing(reason: &'static str) -> Self {
        Self { unavailable: vec![reason], ..Self::default() }
    }
}

struct Reading {
    subject_kind: &'static str,
    subject_id: String,
    metric: &'static str,
    value: f64,
    history: Vec<f64>,
    evidence: Vec<Evidence>,
    comparison: Comparison,
    fixed: Option<Threshold>,
}

#[derive(Clone, Copy)]
enum Comparison { Above, AtLeast, Below, Deviation(f64) }

impl Reading {
    fn new(kind: &'static str, subject: impl Into<String>, metric: &'static str, value: f64, evidence: Vec<Evidence>) -> Self {
        Self { subject_kind: kind, subject_id: subject.into(), metric, value, history: vec![], evidence, comparison: Comparison::Above, fixed: None }
    }

    fn history(mut self, history: Vec<f64>) -> Self {
        self.history = history;
        self
    }

    fn comparison(mut self, comparison: Comparison) -> Self {
        self.comparison = comparison;
        self
    }

    fn fixed(mut self, threshold: Threshold) -> Self {
        self.fixed = Some(threshold);
        self
    }

    fn exceeds(&mut self, threshold: Threshold) -> Option<(f64, f64)> {
        let (baseline, limit) = match self.fixed.unwrap_or(threshold) {
            Threshold::Absolute(limit) => (match self.comparison { Comparison::Deviation(target) => target, _ => limit }, limit),
            Threshold::Baseline { percentile: p, floor } => {
                let baseline = percentile(&mut self.history, p).unwrap_or(floor);
                (baseline, baseline.max(floor))
            }
            Threshold::Change { by } => {
                let baseline = *self.history.first()?;
                let limit = match self.comparison {
                    Comparison::Below => baseline - by,
                    _ => baseline + by,
                };
                (baseline, limit)
            }
        };
        let fires = match self.comparison {
            Comparison::Above => self.value > limit,
            Comparison::AtLeast => self.value >= limit,
            Comparison::Below => self.value < limit,
            Comparison::Deviation(target) => (self.value - target).abs() > limit,
        };
        fires.then_some((baseline, limit))
    }

    fn finding(mut self, threshold: Threshold, window: Window) -> Option<Finding> {
        let (baseline, limit) = self.exceeds(threshold)?;
        if self.evidence.is_empty() {
            return None;
        }
        self.evidence.sort_unstable();
        self.evidence.dedup();
        Some(Finding {
            subject_kind: self.subject_kind, subject_id: self.subject_id, metric: self.metric,
            evidence: self.evidence, measure: Measure {
                value: self.value, baseline, threshold: limit,
                window_start: window.start, window_end: window.end,
            },
        })
    }
}

fn percentile(values: &mut [f64], p: u8) -> Option<f64> {
    values.sort_unstable_by(f64::total_cmp);
    values.get((values.len() * usize::from(p)).div_ceil(100).saturating_sub(1)).copied()
}

#[derive(Clone, Copy)]
struct Window { before: i64, start: i64, end: i64 }

impl Window {
    fn of(at: i64, days: u16) -> Self {
        let length = i64::from(days) * DAY;
        Self { before: at - 2 * length, start: at - length, end: at }
    }

    fn current(self, at: i64) -> bool { at >= self.start }
}

struct Query<'a, 'conn> {
    tx: &'a Transaction<'conn>,
    project: &'a str,
    detector: &'a Detector,
    window: Window,
}

/// Validate every rule before opening the store. The catalog owns all SQL and all identifiers used in SQL.
pub(crate) fn evaluate(store: &Store, project: &str, at: i64, detectors: &[Detector]) -> Result<Evaluation, String> {
    let mut rules = BTreeSet::new();
    for d in detectors {
        if !d.valid() || !rules.insert(d.rule) {
            return Err(format!("Invalid or duplicate Supervisor parameters for {}.", d.rule.id()));
        }
    }
    if !(0..i64::MAX / 1000 - DAY).contains(&at) {
        return Err("Invalid Supervisor evaluation time.".into());
    }
    store.snapshot(SCHEMA, |tx| {
        check_project(tx, project)?;
        let mut checked = Vec::new();
        for detector in detectors {
            let window = Window::of(at, detector.window_days);
            let query = Query { tx, project, detector, window };
            let data = if detector.mode == Mode::Off { Measurements::missing("Detector is off.") } else { detector.rule.query(&query)? };
            let mut findings: Vec<_> = data.readings.into_iter().filter_map(|r| r.finding(detector.threshold, window)).collect();
            findings.sort_by(|a, b| (&a.subject_kind, &a.subject_id, &a.metric).cmp(&(&b.subject_kind, &b.subject_id, &b.metric)));
            checked.push(Checked { rule: detector.rule, mode: detector.mode, scores: detector.scores, unavailable: data.unavailable, findings, complete_metrics: data.complete_metrics });
        }
        Ok(Evaluation { project: project.into(), at, detectors: checked })
    })
}

impl Evaluation {
    pub(crate) fn record(&self, store: &Store) -> Result<(), String> {
        signals::record(store, self)
    }
}

pub(crate) fn signals(store: &Store, project: &str) -> Result<Vec<Signal>, String> {
    store.snapshot(SCHEMA, |tx| signals::list(tx, project))
}
