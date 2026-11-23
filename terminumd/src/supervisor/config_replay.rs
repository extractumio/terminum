// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-253: reproduce the proposed detector over identical retained inputs.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::tasks::ops::{rows, sql};

use super::{Detector, Mode, Rule, WEEK, flow, replay, samples, settings::RuleSettings, signals};

#[derive(Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Receipt {
    pub at: i64,
    pub first: Option<i64>,
    pub source: String,
    pub before: Count,
    pub after: Count,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    anchor_firings: Option<usize>,
    pub labeled_weeks: Vec<i64>,
    pub proof: String,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Count {
    pub firings: usize,
    disabled: bool,
    measured_cutoffs: usize,
    unavailable_cutoffs: usize,
    exact_right: usize,
    exact_wrong: usize,
}

fn source(tx: &Transaction, project: &str, rule: Rule, at: i64) -> Result<(&'static str, Option<i64>), String> {
    match rule {
        Rule::QueueGrowth | Rule::LimitNear | Rule::StalePlan =>
            Ok(("retained snapshots", samples::first_point(tx, project, rule, at)?)),
        Rule::Loop | Rule::Subagents => Ok(("retained counters", samples::first_count(tx, project, rule, at)?)),
        rule if !rule.has_historical_query() => Ok(("no retained historical query", None)),
        _ => {
            let (first, latest) = flow::history_bounds(tx, project, at)?;
            Ok(("retained project history", (latest > 0).then_some(first)))
        }
    }
}

fn labels(tx: &Transaction, project: &str, rule: Rule, first: i64, at: i64) -> Result<Vec<i64>, String> {
    rows(tx, "SELECT DISTINCT observed / ?5 FROM (
        SELECT v.at AS observed FROM supervisor_verdicts v JOIN supervisor_signals s USING(fingerprint)
            WHERE s.project = ?1 AND s.detector = ?2
        UNION ALL SELECT fired_at FROM supervisor_hold_verdicts WHERE project = ?1 AND detector = ?2
        UNION ALL SELECT v.at FROM supervisor_outcomes v JOIN supervisor_actions a ON a.id = v.action
            WHERE a.project = ?1 AND a.detector = ?2 AND v.verdict <> 'undecided'
        ) WHERE observed BETWEEN ?3 AND ?4 ORDER BY 1", (project, rule.id(), first, at, WEEK), |r| r.get(0))
}

fn count(
    tx: &Transaction, project: &str, detector: &Detector, first: i64, at: i64,
    verdicts: &BTreeMap<String, bool>, proof: &mut Sha256,
) -> Result<Count, String> {
    let mut count = Count { disabled: detector.mode == Mode::Off, ..Count::default() };
    let mut exact = BTreeSet::new();
    replay::cutoffs(tx, project, first, at, detector, |end, measured, unavailable, mut findings| {
        count.measured_cutoffs += usize::from(measured);
        count.unavailable_cutoffs += usize::from(!unavailable.is_empty());
        count.firings += findings.len();
        proof.update(signals::encode(&(end, measured, unavailable))?);
        findings.sort_by(|a, b| a.key().cmp(&b.key()));
        for finding in findings {
            let key = replay::firing_key(project, detector.rule, &finding)?;
            proof.update(signals::encode(&(&key, &finding.measure))?);
            if let Some(&right) = verdicts.get(&key)
                && exact.insert(key)
            {
                count.exact_right += usize::from(right);
                count.exact_wrong += usize::from(!right);
            }
        }
        Ok(())
    })?;
    Ok(count)
}

fn anchor(tx: &Transaction, project: &str, detector: &Detector) -> Result<Option<Detector>, String> {
    let rule = detector.rule;
    let default = Detector::for_rule(rule);
    let measure = |config: Option<RuleSettings>| {
        let config = config.unwrap_or_default();
        (config.threshold.unwrap_or(default.threshold), config.window_days.unwrap_or(default.window_days))
    };
    let mut current = (detector.threshold, detector.window_days);
    let mut changes = tx.prepare_cached("SELECT previous, next, class FROM supervisor_config_changes
        WHERE project = ?1 AND detector = ?2 AND applied_at IS NOT NULL AND reverted_at IS NULL
        ORDER BY applied_revision DESC").map_err(sql)?;
    let mut changes = changes.query((project, rule.id())).map_err(sql)?;
    while let Some(row) = changes.next().map_err(sql)? {
        let next = measure(signals::decode(row, 1).map_err(sql)?);
        if next != current { break; }
        let previous = measure(signals::decode(row, 0).map_err(sql)?);
        if row.get::<_, String>(2).map_err(sql)? == "owner" && previous != next { break; }
        current = previous;
    }
    Ok((current != (detector.threshold, detector.window_days)).then(|| Detector {
        threshold: current.0, window_days: current.1, ..detector.clone()
    }))
}

pub(super) fn run(
    tx: &Transaction, project: &str, at: i64, revision: i64, proposal: &impl Serialize,
    before: &Detector, after: &Detector,
) -> Result<Receipt, String> {
    let rule = before.rule;
    let (source, first) = source(tx, project, rule, at)?;
    let mut proof = Sha256::new();
    proof.update(signals::encode(&(project, rule.id(), at, first, revision, proposal))?);
    let (old, new, anchor_firings, labeled_weeks) = if let Some(first) = first {
        let verdicts = rows(tx, "SELECT firing, was_right FROM supervisor_hold_verdicts
            WHERE project = ?1 AND detector = ?2 AND fired_at BETWEEN ?3 AND ?4 ORDER BY fired_at, firing",
            (project, rule.id(), first, at), |r| Ok((r.get(0)?, r.get(1)?)))?.into_iter().collect();
        let old = count(tx, project, before, first, at, &verdicts, &mut proof)?;
        let same = (before.threshold, before.window_days, &before.allocation, before.mode == Mode::Off)
            == (after.threshold, after.window_days, &after.allocation, after.mode == Mode::Off);
        let new = if same { old.clone() } else { count(tx, project, after, first, at, &verdicts, &mut proof)? };
        let anchor_firings = if same { None } else {
            anchor(tx, project, before)?.map(|detector|
                count(tx, project, &detector, first, at, &verdicts, &mut proof).map(|count| count.firings)
            ).transpose()?
        };
        (old, new, anchor_firings, labels(tx, project, rule, first, at)?)
    } else {
        (Count::default(), Count::default(), None, Vec::new())
    };
    proof.update(signals::encode(&(&old, &new, anchor_firings, &labeled_weeks))?);
    Ok(Receipt { at, first, source: source.into(), before: old, after: new, anchor_firings, labeled_weeks,
        proof: crate::auth::crypto::hex(&proof.finalize()) })
}

impl Receipt {
    pub(super) fn measured(&self) -> bool {
        (self.before.measured_cutoffs != 0 || self.before.disabled)
            && (self.after.measured_cutoffs != 0 || self.after.disabled)
            && !(self.before.disabled && self.after.disabled)
    }

    pub(super) fn quiets(&self) -> bool {
        let baseline = self.before.firings.max(self.anchor_firings.unwrap_or_default());
        self.after.firings as u128 * 10 < baseline as u128 * 7
    }

    pub(super) fn summary(&self) -> String {
        let span = self.first.map_or_else(|| "no retained span".into(), |first| format!("{first}–{}", self.at));
        let quality = if self.before.exact_right + self.before.exact_wrong + self.after.exact_right + self.after.exact_wrong == 0 {
            "Quality unmeasured. No exact replayed firing had a retained judgment.".into()
        } else {
            format!("Exact judgments: {} right and {} wrong before, {} right and {} wrong after.",
                self.before.exact_right, self.before.exact_wrong, self.after.exact_right, self.after.exact_wrong)
        };
        let anchor = self.anchor_firings.map_or_else(String::new, |n| format!(" Before automatic tuning: {n} firings."));
        format!("Replay over {} ({span} UTC): {} → {} firings at weekly cutoffs. {} labelled weeks. {quality} {} → {} cutoffs with unavailable inputs.{anchor}",
            self.source, self.before.firings, self.after.firings, self.labeled_weeks.len(),
            self.before.unavailable_cutoffs, self.after.unavailable_cutoffs)
    }
}
