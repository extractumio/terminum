// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213, AC-245: `flow.cost_outlier`'s measured escalate rule. A task's current marked run whose own priced spend
//! is decisively past its type's norm and a large share of the project's week raises the signal's impact to 3, so
//! code stops that run without a round (`holds::automatic`). It is judged on the present only, never in replay or
//! weekly samples, and lapses at the next evaluation once the run no longer works: the recorded scores return to
//! the measured ones. Usage arrives as turns end, so the stop acts between turns, never inside one.

use rusqlite::Transaction;
use serde::{Deserialize, Serialize};

use super::{WEEK, spend::usage_reports};

pub(super) const RULE: &str = "runaway_run";
/// The run's own spend must reach this many times its type's threshold (the p95 of earlier tasks, or the floor).
const TIMES_THRESHOLD: f64 = 3.0;
/// ... and this share of the project's priced spend in the seven days to the evaluation.
const WEEK_SHARE: f64 = 0.10;
/// ... and these dollars, so a small project's few dollars never stop a run.
const FLOOR_DOLLARS: f64 = 20.0;
/// Earlier tasks of the type behind the threshold: with fewer, a p95 is just the largest.
const BASELINE_TASKS: usize = 20;
/// The run's last usage report is at most this old: it is still spending.
const FRESH_SECONDS: i64 = 15 * 60;

/// Unpriced usage makes the week's priced total a lower bound, which would overstate the run's share.
pub(super) const UNPRICED_WEEK: &str =
    "A current run may be a runaway, but some of the week's usage reports have no price, so code does not raise it to level 3.";

/// Which score the rule raised, from what, and the run it names with the numbers that decided it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Raised {
    pub score: String,
    pub from: u8,
    pub rule: String,
    pub run: String,
    pub run_dollars: f64,
    pub week_dollars: f64,
}

/// A run of the current reading: its own priced spend and its last report's time.
pub(super) struct RunSpend<'a> {
    pub run: &'a str,
    pub dollars: f64,
    pub last: i64,
}

/// `Ok(None)`: no raise; `Err` holds why a candidate could not be judged (said as the detector's unavailable input).
pub(super) fn judge(tx: &Transaction, project: &str, at: i64, task: &str, runs: &[RunSpend<'_>], threshold: f64,
    baseline: usize) -> Result<Result<Option<Raised>, &'static str>, String>
{
    if baseline < BASELINE_TASKS { return Ok(Ok(None)); }
    let Some((_, marked)) = super::holds::marked_run_of(tx, project, task)? else { return Ok(Ok(None)) };
    let Some(run) = runs.iter().find(|r| r.run == marked.id) else { return Ok(Ok(None)) };
    if run.last < at - FRESH_SECONDS || run.dollars < FLOOR_DOLLARS.max(TIMES_THRESHOLD * threshold) {
        return Ok(Ok(None));
    }
    let mut week = 0.0;
    for report in usage_reports(tx, project, at - WEEK, at)? {
        let Some(dollars) = report.dollars else { return Ok(Err(UNPRICED_WEEK)) };
        week += dollars;
    }
    if run.dollars < WEEK_SHARE * week { return Ok(Ok(None)); }
    Ok(Ok(Some(Raised { score: "impact".into(), from: 0, rule: RULE.into(), run: marked.id, run_dollars: run.dollars,
        week_dollars: week })))
}
