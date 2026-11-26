// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: `p0_waiting`, the measured escalate rule of `flow.starved` and `product.user_bug_age`. A P0 task still in
//! TODO past the project's level-2 hours raises impact to 2, and to 3 once its further level-3 hours pass, whatever
//! its share of the week's open P0 and P1 task-days; a P1 task keeps its share. Both detectors are present-only
//! (`Rule::present_only`), so neither replay nor a weekly sample raises; the raise names what held the task, so the
//! owner knows what to fix.

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::dispatcher::pool::Slots;
use crate::tasks::{agents::{self, Agent}, auto, ops::sql};

use super::{Query, Reading, Threshold, scoring::{Raise, Score}};

const RULE: &str = "p0_waiting";
/// The most hours either setting takes: 30 days.
const MOST_HOURS: f64 = 720.0;

/// The project's hours after which a P0 task waiting in TODO reaches level 2, and the further hours to level 3.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Hours {
    level2_hours: f64,
    level3_hours: f64,
}

impl Default for Hours {
    fn default() -> Self {
        Self { level2_hours: 24.0, level3_hours: 24.0 }
    }
}

impl Hours {
    pub(super) fn validate(&self) -> Result<(), String> {
        let whole = |h: f64| h.fract() == 0.0 && (1.0..=MOST_HOURS).contains(&h);
        if whole(self.level2_hours) && whole(self.level3_hours) { Ok(()) } else {
            Err("P0 waiting escalation: use whole hours from 1 to 720.".into())
        }
    }

    /// The ages, in minutes, at which a waiting P0 task reaches level 2 and level 3.
    fn minutes(self) -> (f64, f64) {
        (self.level2_hours * 60.0, (self.level2_hours + self.level3_hours) * 60.0)
    }
}

/// A task's blockers that are not DONE, as `l` and `b`; the caller names the task by `l.task`.
pub(super) const OPEN_BLOCKERS: &str = "task_links l JOIN tasks b ON b.id = l.other
    WHERE l.kind = 'blocked_by' AND (b.status <> 'DONE' OR b.archived_at IS NOT NULL)";

/// The reading of a task, raised when `waiting_p0` (a P0 task in TODO) and past the project's level-2 hours: impact 2,
/// or 3 past its further level-3 hours, naming what held the task; `named` is the agent the task names. A waiting P0
/// task's reading fires from its level-2 age when that is below the detector's own limit.
pub(super) fn judge(q: &Query<'_, '_>, mut reading: Reading, waiting_p0: bool, named: Option<&str>) -> Result<Reading, String> {
    if !waiting_p0 { return Ok(reading) }
    let (second, third) = q.detector.p0_hours.minutes();
    if let Threshold::Absolute(limit) = q.detector.threshold && second < limit {
        reading = reading.fixed(Threshold::Absolute(second));
    }
    if reading.value <= second { return Ok(reading) }
    let to = if reading.value >= third { 3 } else { 2 };
    let reason = Some(held(q, &reading.subject_id, named)?);
    reading.score_raises.push(Raise { score: Score::Impact, from: 0, to, rule: RULE.into(), reason });
    Ok(reading)
}

/// Whether one of the project's `agents` that may run task `id` (the one it names, else any) has a free slot.
pub(super) fn free_slot(tx: &Transaction, slots: &Slots, agents: &[Agent], project: &str, id: &str, named: Option<&str>)
    -> Result<bool, String>
{
    for agent in agents.iter().filter(|a| named.is_none_or(|n| a.id == n)) {
        if slots.check_for(tx, agent, project, Some(id), None)?.is_none() {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Why P0 task `id` in TODO has not started, the first that holds: its blocker, Auto-Dispatch's state, its slots,
/// Auto-Dispatch's terms for a candidate.
fn held(q: &Query<'_, '_>, id: &str, named: Option<&str>) -> Result<String, String> {
    let blocker = q.tx.query_row(&format!("SELECT coalesce(b.slug, b.id) FROM {OPEN_BLOCKERS} AND l.task = ?1
        ORDER BY l.place LIMIT 1"), [id], |r| r.get::<_, String>(0)).optional().map_err(sql)?;
    if let Some(name) = blocker {
        return Ok(format!("It waits on {name}, which is not DONE."));
    }
    let setting = auto::setting(q.tx, q.project)?;
    if !setting.on {
        return Ok(setting.off_why.map_or_else(|| "Auto-Dispatch is off for this project.".into(),
            |why| format!("Auto-Dispatch turned itself off: {why}")));
    }
    if !setting.severities.iter().any(|s| s == "P0") {
        return Ok("Auto-Dispatch takes no P0 tasks.".into());
    }
    let at = q.window.end;
    if auto::started_in_day(q.tx, q.project, at)? >= setting.day_most {
        return Ok(format!("Auto-Dispatch started its {} tasks of 24 hours.", setting.day_most));
    }
    if !free_slot(q.tx, &Slots::read(q.tx)?, &agents::of_project(q.tx, q.project)?, q.project, id, named)? {
        return Ok("No agent that may run it has a free slot.".into());
    }
    let slug: String = q.tx.query_row("SELECT coalesce(slug, id) FROM tasks WHERE id = ?1", [id], |r| r.get(0)).map_err(sql)?;
    Ok(if auto::candidates(q.tx, q.project, &setting, at, Some(&slug))?.is_empty() {
        "Auto-Dispatch passes over it: it started it before, a round left it, it names no agent in the pool, \
            it changed within the wait, or a session, run, wait or its main task holds it."
    } else { "Auto-Dispatch is on and has not started it." }.into())
}

#[cfg(test)]
#[path = "waiting_tests.rs"]
mod tests;
