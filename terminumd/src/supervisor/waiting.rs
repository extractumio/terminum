// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-213: `p0_waiting`, the measured escalate rule of `flow.starved` and `product.user_bug_age`. A P0 task still in
//! TODO past its age limit raises impact to 2, and to 3 a full day past it, whatever its share of the week's open
//! P0 and P1 task-days; a P1 task keeps its share. Both detectors are present-only (`Rule::present_only`), so neither
//! replay nor a weekly sample raises; the raise names what held the task, so the owner knows what to fix.

use rusqlite::{OptionalExtension, Transaction};

use crate::dispatcher::pool::Slots;
use crate::tasks::{agents::Agent, auto, ops::sql};

use super::{Query, Reading, scoring::{Raise, Score}};

pub(super) const RULE: &str = "p0_waiting";
/// Minutes past the age limit after which a waiting P0 task is critical: a full day.
const CRITICAL_PAST: f64 = 24.0 * 60.0;

/// A P0 task's reading while it is in TODO: past its limit it raises impact to 2, or 3 a day past it, naming what
/// `held` the task; only then is that read.
pub(super) fn judge(q: &Query<'_, '_>, reading: &mut Reading, held: impl FnOnce() -> Result<String, String>) -> Result<(), String> {
    let Some((_, limit)) = reading.exceeds(q.detector.threshold) else { return Ok(()) };
    let to = if reading.value - limit >= CRITICAL_PAST { 3 } else { 2 };
    reading.score_raises.push(Raise::to(Score::Impact, to, RULE, held()?));
    Ok(())
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

/// The first blocker of task `id` that is not DONE, by its name.
pub(super) fn blocker(tx: &Transaction, id: &str) -> Result<Option<String>, String> {
    tx.query_row("SELECT coalesce(b.slug, b.id) FROM task_links l JOIN tasks b ON b.id = l.other
        WHERE l.task = ?1 AND l.kind = 'blocked_by' AND (b.status <> 'DONE' OR b.archived_at IS NOT NULL)
        ORDER BY l.place LIMIT 1", [id], |r| r.get(0)).optional().map_err(sql)
}

/// Why a P0 task in TODO has not started, the first that holds: its blocker, Auto-Dispatch's state, its slots.
pub(super) fn held(tx: &Transaction, project: &str, at: i64, blocker: Option<String>, free: bool) -> Result<String, String> {
    if let Some(name) = blocker {
        return Ok(format!("It waits on {name}, which is not DONE."));
    }
    let setting = auto::setting(tx, project)?;
    if !setting.on {
        return Ok(setting.off_why.map_or_else(|| "Auto-Dispatch is off for this project.".into(),
            |why| format!("Auto-Dispatch turned itself off: {why}")));
    }
    if !setting.severities.iter().any(|s| s == "P0") {
        return Ok("Auto-Dispatch takes no P0 tasks.".into());
    }
    if auto::started_in_day(tx, project, at)? >= setting.day_most {
        return Ok(format!("Auto-Dispatch started its {} tasks of 24 hours.", setting.day_most));
    }
    Ok(if free { "Auto-Dispatch is on and has not started it." } else { "No agent that may run it has a free slot." }.into())
}

#[cfg(test)]
#[path = "waiting_tests.rs"]
mod tests;
