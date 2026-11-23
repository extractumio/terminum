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
    move_marked(tx, author, id, status, None, at)
}

/// What a move's history says it was for, its field kept on the move: (field, words) (terminum-dispatcher.md AC-264).
pub type Mark<'a> = Option<(&'a str, &'a str)>;

/// `move_to`, its history saying why by `mark`.
pub fn move_marked(tx: &Transaction, author: &str, id: &str, status: &str, mark: Mark, at: i64) -> Result<(), String> {
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
    // a batch's integration task ends with its batch (terminum-integration.md AC-194): its keys decide, not a move
    // Dispatcher's own moves of it: its run starting and waiting for a human
    let runs_it = author == super::store::DISPATCHER && ["IN PROGRESS", "WAIT FOR HUMAN"].contains(&status);
    if let Some(n) = crate::integrate::store::live_by_task(tx, &task.key)? && !runs_it {
        return Err(format!("{} is batch {n}'s integration task: it ends with its batch.", task.id));
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
    place_marked(tx, author, &task, status, mark, at)
}

/// `task` in `status`, its history saying who moved it; a task that leaves INTEGRATION leaves the queue, whoever
/// moved it (terminum-integration.md AC-187).
pub(crate) fn place(tx: &Transaction, author: &str, task: &Task, status: &str, at: i64) -> Result<(), String> {
    place_marked(tx, author, task, status, None, at)
}

fn place_marked(tx: &Transaction, author: &str, task: &Task, status: &str, mark: Mark, at: i64) -> Result<(), String> {
    if task.status == INTEGRATION {
        super::integration_queue::leave(tx, task, status, at)?;
        // A return is activity (AC-123, AC-198), even if no walk saw the wait; keep the budget already used.
        if status == "IN PROGRESS" && let Some(mut run) = crate::dispatcher::runs::holding(tx, &task.key)? {
            run.idle_for = 0;
            run.nudged = 0;
            run.nudge_turn = false;
            run.last_walk = at;
            crate::dispatcher::runs::save(tx, &run)?;
        }
    }
    let rank = top_rank(tx, status, &task.severity)?;
    tx.execute("UPDATE tasks SET status = ?1, rank = ?2, updated_at = ?3 WHERE id = ?4", (status, rank, at, &task.key)).map_err(sql)?;
    let who = user_name(tx, author)?;
    let mut data = json!({"from": task.status, "to": status});
    // the run it failed in, its latest, ended or not: a launch later in its second is not that run (terminum-supervisor.md §9)
    if status == "FAILED" && let Some(run) = crate::dispatcher::runs::last_of(tx, &task.key)? {
        data["run"] = json!(run.id);
    }
    let mut words = format!("{who} moved it from {} to {status}", task.status);
    if let Some((field, why)) = mark {
        data[field] = json!(true);
        words = format!("{words}: {why}");
    }
    log(tx, &task.key, author, "move", &words, Some(&data), at)?;
    // however it ended, what its session made stays named on it (terminum-dispatcher.md AC-166)
    if matches!(status, "DONE" | "FAILED") && let Some(line) = super::closeout::made_line(tx, task)? {
        log(tx, &task.key, super::store::DISPATCHER, "told", &line, None, at)?;
    }
    Ok(())
}
