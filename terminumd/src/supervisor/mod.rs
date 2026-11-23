// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Deterministic Supervisor queries (AC-211, AC-212). Evaluation is read-only; recording is explicit.

pub(crate) mod action_tasks;
mod attach;
pub(crate) mod breakers;
mod outcomes;
pub(crate) mod misses;
mod measures;
#[cfg(test)]
mod measures_tests;
pub(crate) mod audit;
pub(crate) mod reflection;
mod reflection_timeline;
#[cfg(test)]
mod reflection_tests;
mod outcome_evidence;
mod attribution;
mod catalog;
mod decisions;
pub(crate) mod config_changes;
mod config_replay;
mod config_lifecycle;
mod config_engine;
mod config_api;
#[cfg(test)]
mod config_tests;
mod nudges;
mod digest;
pub(crate) mod review;
mod review_data;
mod impact;
pub(crate) mod holds;
pub(crate) mod holds_api;
#[cfg(test)]
mod holds_tests;
mod incidents;
pub(crate) mod thread;
pub(crate) mod talk;
mod incidents_api;
pub(crate) mod line;
mod line_api;
mod line_command;
mod probe_command;
mod probe_git;
mod probe_arch;
mod probe_gate;
pub(crate) mod probes;
mod outside_feedback;
mod outside_indicators;
mod probe_rules;
mod integration_rules;
mod line_queue;
mod replay;
pub(crate) mod runtime;
mod samples;
mod sensors;
mod scoreboard;
mod screen;
mod flow;
mod people;
mod signals;
pub(crate) mod rounds;
pub(crate) mod round_tools;
mod round_evidence;
#[cfg(test)]
mod communications_tests;
mod evaluation;
mod settings;
mod settings_api;
pub(crate) mod surfaces;
pub(crate) mod design_gates;
pub(crate) mod design_reviews;
pub(crate) mod verify_review;
mod spend;
mod rate_intervals;
pub(crate) mod governor;
pub(crate) mod register;
pub(crate) mod fix_tasks;
pub(crate) mod task_policy;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod catalog_tests;
#[cfg(test)]
mod replay_tests;
#[cfg(test)]
mod incidents_tests;

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::tasks::store::Store;
#[cfg(test)]
mod register_tests;

pub(crate) use catalog::Rule;
pub(crate) use signals::Signal;
pub(crate) use settings::{Level, level};
pub(crate) use replay::replay;
#[cfg(test)]
pub(crate) use evaluation::evaluate;
use evaluation::{evaluate_snapshot, measure_detector, collect_findings};

pub(crate) fn task_created(tx: &rusqlite::Transaction, author: &str, task: &crate::tasks::ops::Task,
    words: &str, at: i64) -> Result<(), String>
{
    misses::owner_bug(tx, author, task, at)?;
    design_gates::created(tx, author, task, words, at)
}

const SCHEMA: usize = crate::tasks::store::SCHEMA_VERSION;

fn dns_labels(host: &str) -> bool {
    host.split('.').all(|label| {
        !label.is_empty() && label.len() <= 63 && !label.starts_with('-') && !label.ends_with('-')
            && label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}
const DAY: i64 = 86_400;
const WEEK: i64 = 7 * DAY;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Off,
    Shadow,
    Live,
    Digest,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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
        Rule::named(name).map(Self::for_rule)
    }

    fn for_rule(rule: Rule) -> Self {
        let threshold = rule.defaults();
        let window_days = if rule == Rule::FailedWithoutReason { 7 } else { 28 };
        Self { rule, mode: Mode::Off, window_days, threshold, allocation: BTreeMap::new(), scores: Scores::default() }
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
    SupervisorPoints(i64),
    SupervisorProbes(i64),
    IntegrationRound { batch: i64, round: i64 },
    IntegrationVerify(String),
    SupervisorHookCounts(i64),
    SupervisorHeartbeat { project: String, last_ok: Option<i64> },
    SupervisorRounds(String),
    SupervisorActions(i64),
    SupervisorLine { channel: i64, failure: String },
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
    attributed_runs: BTreeSet<String>,
    source: impact::Source,
    charter_breach: bool,
    assessment: impact::Assessment,
    scores: Scores,
}

impl Finding {
    fn key(&self) -> (&str, &str, &str) {
        (self.subject_kind, &self.subject_id, self.metric)
    }
}

#[derive(Debug)]
pub(crate) struct Evaluation {
    project: String,
    at: i64,
    pub(crate) detectors: Vec<Checked>,
}

type MeasuredSubjects = Option<Vec<(&'static str, String, &'static str)>>;

#[derive(Debug)]
pub(crate) struct Checked {
    pub rule: Rule,
    pub mode: Mode,
    pub unavailable: Vec<&'static str>,
    pub findings: Vec<Finding>,
    thresholds: BTreeMap<&'static str, attach::Range>,
    complete_metrics: Vec<&'static str>,
    measured: MeasuredSubjects,
    observations: signals::Observations,
}

/// A detector returns measured subjects; an unavailable input cannot prove anything resolved.
#[derive(Default)]
struct Measurements {
    readings: Vec<Reading>,
    unavailable: Vec<&'static str>,
    complete_metrics: Vec<&'static str>,
    threshold_hint: Option<(&'static str, f64)>,
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
    attributed_runs: BTreeSet<String>,
    source: impact::Source,
}

#[derive(Clone, Copy, Debug)]
enum Comparison { Above, AtLeast, Below, Deviation(f64) }

impl Reading {
    fn new(kind: &'static str, subject: impl Into<String>, metric: &'static str, value: f64, evidence: Vec<Evidence>) -> Self {
        Self { subject_kind: kind, subject_id: subject.into(), metric, value, history: vec![], evidence, comparison: Comparison::Above, fixed: None, attributed_runs: BTreeSet::new(), source: Default::default() }
    }

    fn source(mut self, source: impact::Source) -> Self {
        self.source = source;
        self
    }

    fn attributed_runs(mut self, runs: BTreeSet<String>) -> Self {
        self.attributed_runs = runs;
        self
    }

    fn key(&self) -> (&str, &str, &str) {
        (self.subject_kind, &self.subject_id, self.metric)
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

    fn limits(&self, threshold: Threshold) -> Option<(f64, f64)> {
        Some(match self.fixed.unwrap_or(threshold) {
            Threshold::Absolute(limit) => (match self.comparison { Comparison::Deviation(target) => target, _ => limit }, limit),
            Threshold::Baseline { percentile: p, floor } => {
                let baseline = percentile(&mut self.history.clone(), p).unwrap_or(floor);
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
        })
    }

    fn exceeds(&self, threshold: Threshold) -> Option<(f64, f64)> {
        let (baseline, limit) = self.limits(threshold)?;
        self.comparison.fires(self.value, limit).then_some((baseline, limit))
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
            evidence: self.evidence, attributed_runs: self.attributed_runs, source: self.source, charter_breach: false,
            assessment: Default::default(), scores: Scores::default(), measure: Measure {
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

    fn week(end: i64) -> Self {
        let start = end - WEEK + 1;
        Self { before: start - WEEK, start, end }
    }

    fn current(self, at: i64) -> bool { at >= self.start }
}

struct Query<'a, 'conn> {
    tx: &'a Transaction<'conn>,
    project: &'a str,
    detector: &'a Detector,
    window: Window,
    historical: bool,
}

#[derive(Default)]
struct EvaluationInputs<'a> {
    first: Option<i64>,
    weeks: Option<&'a [scoreboard::Week]>,
    current_state: bool,
    observe_thresholds: bool,
}

impl Evaluation {
    fn critical(project: &str, rule: Rule, reading: Reading, window: Window) -> Option<Self> {
        let mut finding = reading.finding(rule.defaults(), window)?;
        finding.scores = Scores { impact: 3, ..Scores::default() };
        Some(Self { project: project.into(), at: window.end, detectors: vec![Checked {
            rule, mode: Mode::Live, unavailable: Vec::new(), findings: vec![finding],
            thresholds: Default::default(), complete_metrics: Vec::new(), measured: None, observations: Default::default(),
        }] })
    }

    pub(crate) fn record(&self, store: &Store) -> Result<(), String> {
        signals::record(store, self)
    }
}

pub(crate) fn signals(store: &Store, project: &str) -> Result<Vec<Signal>, String> {
    store.snapshot(SCHEMA, |tx| signals::list(tx, project))
}

#[cfg(test)]
mod design_gates_tests;
