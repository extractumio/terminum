// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-246: decisions keep their original rule and target. Each check adds one observation.

use rusqlite::{OptionalExtension, Transaction, params};
use serde::Serialize;
use std::collections::BTreeSet;

use crate::tasks::ops::{rows, sql};
use super::{Evidence, Signal, breakers, outcome_evidence, signals};

#[derive(Clone, Copy)]
pub(super) enum Kind { Hold, Fix, Run, Alert, Cause, Card, Nudge }

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Hold => "hold", Self::Fix => "fix", Self::Run => "run", Self::Alert => "alert",
            Self::Cause => "cause", Self::Card => "card", Self::Nudge => "nudge",
        }
    }

    fn read(name: &str) -> Self {
        match name {
            "hold" => Self::Hold, "fix" => Self::Fix, "run" => Self::Run, "alert" => Self::Alert,
            "cause" => Self::Cause, "card" => Self::Card, "nudge" => Self::Nudge,
            _ => unreachable!("action kind checked by the store"),
        }
    }
}

pub(super) struct Action {
    pub id: i64,
    pub detector: String,
    pub response: String,
    pub kind: Kind,
    pub source: String,
    pub fingerprint: String,
    pub target: String,
    pub at: i64,
    pub baseline: String,
}

const ACTION: &str = "a.id,a.detector,a.response,a.kind,a.source,a.fingerprint,a.target,a.at,a.baseline";

fn read(row: &rusqlite::Row<'_>) -> rusqlite::Result<Action> {
    Ok(Action { id: row.get(0)?, detector: row.get(1)?, response: row.get(2)?,
        kind: Kind::read(&row.get::<_, String>(3)?), source: row.get(4)?, fingerprint: row.get(5)?,
        target: row.get(6)?, at: row.get(7)?, baseline: row.get(8)? })
}

pub(super) fn record(tx: &Transaction, signal: &Signal, kind: Kind, source: &str, target: &str,
    baseline: &str, at: i64) -> Result<(), String>
{
    record_rule(tx, (signal, signal.response), kind, source, target, baseline, at)
}

pub(super) fn record_rule(tx: &Transaction, (signal, response): (&Signal, &str), kind: Kind, source: &str, target: &str,
    baseline: &str, at: i64) -> Result<(), String>
{
    tx.execute("INSERT INTO supervisor_actions (project, detector, response, kind, source, fingerprint, target, at, baseline)
        VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT (project, kind, source) DO NOTHING",
        params![signal.project, signal.detector, response, kind.name(), source, signal.fingerprint, target, at, baseline])
        .map(drop).map_err(sql)
}

pub(super) fn unattributed(tx: &Transaction, project: &str, kind: Kind, source: &str, target: &str, at: i64)
    -> Result<(), String>
{
    tx.execute("INSERT INTO supervisor_actions (project, detector, response, kind, source, fingerprint, target, at)
        VALUES (?1,'unattributed','',?2,?3,'',?4,?5) ON CONFLICT (project,kind,source) DO NOTHING",
        (project, kind.name(), source, target, at)).map(drop).map_err(sql)
}

pub(super) fn find(tx: &Transaction, project: &str, kind: Kind, source: &str) -> Result<Option<Action>, String> {
    tx.query_row(&format!("SELECT {ACTION} FROM supervisor_actions a
        WHERE project = ?1 AND kind = ?2 AND source = ?3"), (project, kind.name(), source), read).optional().map_err(sql)
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Verdict { Right, Wrong, Undecided }

impl Verdict {
    fn name(self) -> &'static str {
        match self { Self::Right => "right", Self::Wrong => "wrong", Self::Undecided => "undecided" }
    }
}

pub(super) type Judgment = (Verdict, &'static str);

pub(super) fn judge(tx: &Transaction, project: &str, action: &Action, days: i64, at: i64, judgment: Judgment)
    -> Result<bool, String>
{
    let (verdict, evidence) = judgment;
    let changed = tx.execute("INSERT INTO supervisor_outcomes VALUES (?1,?2,?3,?4,?5)
        ON CONFLICT (action, days) DO NOTHING", (action.id, days, at, verdict.name(), evidence)).map_err(sql)?;
    if changed == 0 { return Ok(false); }
    if verdict == Verdict::Wrong {
        breakers::wrong(tx, project, action, matches!(action.kind, Kind::Alert) && days == 0, at)?;
    }
    if days == 0 {
        report_unjudged(tx, project, &action.detector, &action.response, at)?;
    } else {
        let next = match days { 1 => Some(7), 7 => Some(28), _ => None };
        tx.execute("UPDATE supervisor_actions SET next_days = ?2 WHERE id = ?1 AND next_days = ?3",
            (action.id, next, days)).map_err(sql)?;
    }
    Ok(true)
}

pub(super) fn check(tx: &Transaction, project: &str, at: i64) -> Result<(), String> {
    let mut changed = BTreeSet::new();
    for days in [1, 7, 28] {
        let due = rows(tx, &format!("SELECT {ACTION} FROM supervisor_actions a
            WHERE a.project = ?1 AND a.next_days = ?2 AND a.due_at <= ?3
            ORDER BY a.due_at, a.id LIMIT 100"), (project, days, at), read)?;
        for action in due {
            let result = outcome_evidence::check(tx, project, &action, at)?;
            judge(tx, project, &action, days, at, result)?;
            changed.insert((action.detector, action.response));
        }
    }
    for (detector, response) in changed { report_unjudged(tx, project, &detector, &response, at)?; }
    Ok(())
}

// An action is one sample. The latest scheduled check updates the summary, while
// an explicit wrong verdict and the historical observations remain permanent.
const LATEST: &str = "LEFT JOIN supervisor_outcomes o ON o.action = a.id AND o.days = (
    SELECT v.days FROM supervisor_outcomes v WHERE v.action = a.id ORDER BY (v.days = 0) DESC, v.days DESC LIMIT 1)";

#[derive(Serialize)]
pub(super) struct Score {
    detector: String,
    response: String,
    right: i64,
    wrong: i64,
    undecided: i64,
    pending: i64,
}

pub(super) fn scores(tx: &Transaction, project: &str) -> Result<Vec<Score>, String> {
    rows(tx, &format!("SELECT a.detector, a.response, count(*) FILTER (WHERE o.verdict = 'right'),
        count(*) FILTER (WHERE o.verdict = 'wrong'), count(*) FILTER (WHERE o.verdict = 'undecided'),
        count(*) FILTER (WHERE o.verdict IS NULL)
        FROM supervisor_actions a {LATEST} WHERE a.project = ?1
        GROUP BY a.detector, a.response ORDER BY a.detector, a.response"), [project], |r|
        Ok(Score { detector: r.get(0)?, response: r.get(1)?, right: r.get(2)?, wrong: r.get(3)?,
            undecided: r.get(4)?, pending: r.get(5)? }))
}

fn report_unjudged(tx: &Transaction, project: &str, detector: &str, response: &str, at: i64) -> Result<(), String> {
    let (undecided, judged): (i64, i64) = tx.query_row(&format!("SELECT count(*) FILTER (WHERE o.verdict = 'undecided'),
        count(*) FILTER (WHERE o.verdict IN ('right','wrong')) FROM supervisor_actions a {LATEST}
        WHERE a.project = ?1 AND a.detector = ?2 AND a.response = ?3"), (project, detector, response),
        |r| Ok((r.get(0)?, r.get(1)?))).map_err(sql)?;
    let rule = format!("{detector} / {response}");
    let subject = ("rule", rule.as_str(), "undecided_outcomes");
    if undecided > judged {
        let evidence = rows(tx, &format!("SELECT a.id FROM supervisor_actions a {LATEST}
            WHERE a.project = ?1 AND a.detector = ?2 AND a.response = ?3 AND o.verdict = 'undecided'
            ORDER BY a.id DESC LIMIT 16"), (project, detector, response), |r| Ok(Evidence::SupervisorActions(r.get(0)?)))?;
        signals::diagnostic(tx, project, "self.unjudged", subject, &evidence, (undecided as f64, judged as f64), at)?;
    } else {
        tx.execute("UPDATE supervisor_signals SET state = 'resolved' WHERE project = ?1 AND detector = 'self.unjudged'
            AND subject_kind = 'rule' AND subject_id = ?2 AND metric = 'undecided_outcomes' AND state = 'raised'",
            (project, &rule)).map_err(sql)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "outcomes_tests.rs"]
mod tests;
