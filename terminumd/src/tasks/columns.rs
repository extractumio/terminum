// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A task's move to another column (AC-107), the rules for humans and agents alike; a task in INTEGRATION is
//! put there only by integrate and leaves the queue whatever moves it out (terminum-integration.md AC-187).

use rusqlite::{OptionalExtension, Transaction};
use serde_json::json;

use super::ops::{get, log, pane_of, sql, top_rank, user_name, Task, COLUMNS, INTEGRATION};

/// A session works on one task at a time (AC-113), but for those one run gave it together (terminum-dispatcher.md
/// AC-138): refused while another than `but` stands in IN PROGRESS.
pub(crate) fn check_free(tx: &Transaction, session: &str, but: &str) -> Result<(), String> {
    let still = crate::dispatcher::runs::STILL;
    let other: Option<String> = tx.query_row(&format!("SELECT slug FROM tasks WHERE session_key = ?1 AND status = 'IN PROGRESS' AND id <> ?2 AND archived_at IS NULL
         AND NOT EXISTS (SELECT 1 FROM run_tasks a JOIN run_tasks b ON a.run = b.run JOIN runs r ON r.id = a.run WHERE a.task = tasks.id AND b.task = ?2 AND {still})"), (session, but), |r| r.get(0))
        .optional().map_err(sql)?;
    match other {
        Some(other) => Err(format!("Session {} already works on {other}. Finish or move {other} first.", pane_of(session))),
        None => Ok(()),
    }
}

/// Moves a task to a column, on top of its severity's cards; its history says who did it.
pub fn move_to(tx: &Transaction, author: &str, id: &str, status: &str, at: i64) -> Result<(), String> {
    if !COLUMNS.contains(&status) {
        return Err(format!("Status: one of {}", COLUMNS.join(", ")));
    }
    let task = get(tx, id)?;
    if task.status == status {
        return Ok(());
    }
    if status == INTEGRATION {
        return Err(format!("{} goes to INTEGRATION only by integrate: it queues its commit.", task.id));
    }
    // a schedule's task is a run from its first moment: it waits for a human, is done or failed, and runs again
    if status == "TODO" && let Some(s) = super::schedules::of_task(tx, &task.key)? {
        return Err(format!("{} is a run of schedule {}: it does not go to TODO.", task.id, s.key));
    }
    // Dispatcher's own move follows its run: another's task in that session is that task's matter
    if status == "IN PROGRESS" && author != super::store::DISPATCHER
        && let Some(session) = &task.session {
            check_free(tx, session, &task.key)?;
        }
    place(tx, author, &task, status, at)
}

/// `task` in `status`, its history saying who moved it; a task that leaves INTEGRATION leaves the queue, whoever
/// moved it (terminum-integration.md AC-187).
pub(crate) fn place(tx: &Transaction, author: &str, task: &Task, status: &str, at: i64) -> Result<(), String> {
    if task.status == INTEGRATION {
        super::integration_queue::leave(tx, &task.key)?;
    }
    let rank = top_rank(tx, status, &task.severity)?;
    tx.execute("UPDATE tasks SET status = ?1, rank = ?2, updated_at = ?3 WHERE id = ?4", (status, rank, at, &task.key)).map_err(sql)?;
    let who = user_name(tx, author)?;
    log(tx, &task.key, author, "move", &format!("{who} moved it from {} to {status}", task.status), Some(&json!({"from": task.status, "to": status})), at)?;
    // however it ended, what its session made stays named on it (terminum-dispatcher.md AC-166)
    if matches!(status, "DONE" | "FAILED") && let Some(line) = super::closeout::made_line(tx, task)? {
        log(tx, &task.key, super::store::DISPATCHER, "told", &line, None, at)?;
    }
    Ok(())
}
