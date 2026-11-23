// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-239: board authority and advisory ranks. The board's own rank is never changed.

use rusqlite::{OptionalExtension, Transaction};
use serde::Serialize;

use crate::tasks::ops::{self, Task, sql};

use super::{DAY, Evidence, register, signals};

pub(crate) struct Override {
    event: String,
    at: i64,
    pub author: String,
}

pub(crate) fn override_of(tx: &Transaction, key: &str, at: i64) -> Result<Option<Override>, String> {
    tx.query_row("SELECT id, author, at FROM supervisor_human_actions
        WHERE task = ?1 AND at > ?2 AND at <= ?3 ORDER BY at DESC, event_order DESC LIMIT 1",
        (key, at - DAY, at), |r| Ok(Override { event: r.get(0)?, author: r.get(1)?, at: r.get(2)? }))
        .optional().map_err(sql)
}

pub(crate) fn check(tx: &Transaction, task: &Task, at: i64) -> Result<(), String> {
    if override_of(tx, &task.key, at)?.is_some() {
        Err(format!("A human or manager acted on {}. The Supervisor waits 24 hours.", task.id))
    } else { Ok(()) }
}

/// Commit the diagnostic before surfacing a refusal. The event ID deduplicates repeated attempts.
pub(crate) fn record_override(tx: &Transaction, task: &Task, at: i64) -> Result<Option<String>, String> {
    let Some(o) = override_of(tx, &task.key, at)? else { return Ok(None) };
    signals::diagnostic(tx, &task.project, "self.fights", ("task", &task.id, "human_or_manager_override"),
        &[Evidence::Events(o.event)], (1., 0.), o.at)?;
    Ok(Some(format!("{} acted on {}. The Supervisor waits 24 hours.", o.author, task.id)))
}

pub(crate) fn record_recent(tx: &Transaction, project: &str, at: i64) -> Result<(), String> {
    let touched = ops::rows(tx, "SELECT t.slug FROM tasks t
        WHERE t.project = ?1 AND EXISTS (SELECT 1 FROM supervisor_human_actions e WHERE e.task = t.id AND e.at > ?2)
          AND (EXISTS (SELECT 1 FROM supervisor_task_marks m WHERE m.task = t.id)
            OR EXISTS (SELECT 1 FROM supervisor_rank_proposals p WHERE p.task = t.id)
            OR EXISTS (SELECT 1 FROM events e WHERE e.task = t.id AND e.author = 'supervisor:' || ?1))",
        (project, at - DAY), |r| r.get::<_, String>(0))?;
    for id in touched { record_override(tx, &ops::get(tx, &id)?, at)?; }
    for (id, fingerprint, event, at) in ops::rows(tx, "SELECT action, fingerprint, event, at
        FROM supervisor_task_overrides WHERE project = ?1 AND at > ?2 AND at <= ?3",
        (project, at - DAY, at), |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?)))?
    {
        super::reflection::enqueue(tx, project, ("action", &id.to_string(), Some(&fingerprint)),
            "human_override", &serde_json::json!({"event":event,"actor":"human"}), at)?;
    }
    tx.execute("UPDATE supervisor_signals SET state = 'resolved' WHERE project = ?1
        AND detector = 'self.fights' AND state = 'raised' AND last_seen <= ?2", (project, at - DAY)).map_err(sql)?;
    Ok(())
}

pub(super) fn propose(tx: &Transaction, project: &str, id: i64, task: &Task, run: Option<&str>, at: i64) -> Result<(), String> {
    let problem = register::get(tx, project, id, at)?;
    if task.project != project || !problem.active() || !register::relates(tx, &problem, &task.key)? {
        return Err("A rank requires an open problem and a related task in this project.".into());
    }
    tx.execute("INSERT INTO supervisor_rank_proposals VALUES (?1, ?2, ?3, ?4)
        ON CONFLICT (task) DO UPDATE SET problem = excluded.problem, proposed_at = excluded.proposed_at,
            source_round = excluded.source_round", (&task.key, id, at, run)).map_err(sql)?;
    Ok(())
}

#[derive(Serialize)]
pub(crate) struct Rank {
    pub problem: i64,
    pub impact: u8,
    pub rank: f64,
}

pub(crate) fn rank(tx: &Transaction, project: &str, key: &str, at: i64) -> Result<Option<Rank>, String> {
    let id = tx.query_row("SELECT p.problem FROM supervisor_rank_proposals p
        JOIN supervisor_problems r ON r.id = p.problem AND r.project = ?2 AND r.state NOT IN ('resolved', 'accepted')
        WHERE p.task = ?1 AND (p.source_round IS NULL OR EXISTS (SELECT 1 FROM supervisor_rounds q
            WHERE q.run = p.source_round AND q.finding IS NOT NULL AND q.failure IS NULL))
          AND NOT EXISTS (SELECT 1 FROM supervisor_human_actions e
          WHERE e.task = p.task AND e.at >= p.proposed_at)",
        (key, project), |r| r.get::<_, i64>(0)).optional().map_err(sql)?;
    id.map(|id| {
        let p = register::get(tx, project, id, at)?;
        Ok(p.action_ready.then_some(Rank { problem: id, impact: p.impact, rank: p.rank }))
    }).transpose().map(Option::flatten)
}

pub(crate) fn check_marked(tx: &Transaction, task: &Task, allow: bool) -> Result<bool, String> {
    tx.query_row("SELECT NOT EXISTS (SELECT 1 FROM supervisor_auto_marks m WHERE m.task = ?1 AND NOT (?2 AND m.eligible))",
        (&task.key, allow), |r| r.get(0)).map_err(sql)
}

/// Owner cards are identified as Supervisor work without reserving fix spend.
pub(super) fn mark_card(tx: &Transaction, task: &Task) -> Result<(), String> {
    tx.execute("INSERT OR IGNORE INTO supervisor_task_marks (task, project, estimate) VALUES (?1, ?2, 0)",
        (&task.key, &task.project)).map(drop).map_err(sql)
}
