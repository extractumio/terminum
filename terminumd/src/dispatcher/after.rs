// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A run that waits for other tasks (`terminum-dispatcher.md` AC-210): its agent said so by wait_for_task; the session
//! closes as after a question (AC-157), and a kept press (AC-158) goes on in the same conversation once every task it
//! waits for is DONE, with fixed words and their IDs alone; one of them FAILED, archived or gone asks a human.

use rusqlite::Transaction;

use super::api::Press;
use super::runs::{How, Run};
use crate::tasks::ops::{self, sql, Task};
use crate::tasks::store::{DISPATCHER, HUMAN};

/// Where the tasks a wait names stand.
#[derive(Debug, PartialEq)]
pub enum Blockers {
    /// one is still to be done
    Open,
    /// every one is DONE
    Done,
    /// one will not be DONE: the questions for the human
    Gone(String),
}

/// Where the tasks of store keys `keys`, which task `id` waits for, stand.
pub(super) fn blockers(tx: &Transaction, id: &str, keys: &[String]) -> Result<Blockers, String> {
    let mut open = false;
    for key in keys {
        let Ok(t) = ops::get(tx, key) else { return Ok(Blockers::Gone(gone(id, "a task that was deleted", "deleted"))) };
        let ended = if t.archived_at.is_some() { Some("archived") } else if t.status == "FAILED" { Some("FAILED") } else { None };
        if let Some(how) = ended {
            return Ok(Blockers::Gone(gone(id, &t.id, how)));
        }
        open |= t.status != "DONE";
    }
    Ok(if open { Blockers::Open } else { Blockers::Done })
}

/// The questions when `blocker`, which `id` waited for, ended `how`, in AC-157's three parts.
fn gone(id: &str, blocker: &str, how: &str) -> String {
    format!("Why you are needed: {id} waited for {blocker}, which is {how}, so Dispatcher cannot go on with {id} by itself.\n\
        Recommended:\n1. RUN {id} again with your answer: whether it goes on without {blocker}, and how; its conversation goes on. Other options: run {blocker} again first, or archive {id}.\n\
        Details: {id}'s agent said it could not go on before {blocker} was DONE (Blocked by on its card says why). Dispatcher goes on with a waiting task by itself only once every task it waits for is DONE; {blocker} is {how} instead.")
}

/// The wait of store key `key` (task `id`) ends: `words` ask a human, and it waits for them (AC-157's RUN goes on).
pub(super) fn ask(tx: &Transaction, key: &str, id: &str, words: &str) -> Result<(), String> {
    super::waits::remove(tx, key)?;
    let at = ops::now();
    ops::questions(tx, DISPATCHER, id, words, at)?;
    ops::move_to(tx, DISPATCHER, id, "WAIT FOR HUMAN", at)
}

/// What the agent hears through the gate once the tasks `after` it waited for are DONE: fixed words with IDs.
pub(super) fn go_on(id: &str, after: &[String]) -> String {
    let (them, are) = if after.len() == 1 { (after[0].clone(), "is") } else { (after.join(", "), "are") };
    format!("[terminum Dispatcher] {id}: {them}, which it waited for, {are} DONE, and Dispatcher resumed this conversation. \
        Go on with {id}: bring in their work (merge the project's branch where they landed), run your checks again, then finish it as you were asked.")
}

/// The words of a session closed while its task waits for other tasks.
pub const CLOSED: &str = "Session closed: it waits for other tasks. Dispatcher resumes this conversation once they are DONE.";

/// Task `t`, which `run` holds, now waits for `blockers` (tasks of its project, none DONE, FAILED or archived): its
/// press is kept, as who started the run and how (AC-183), to go on once they are DONE.
pub(crate) fn keep(tx: &Transaction, run: &Run, t: &Task, blockers: &[Task], at: i64) -> Result<(), String> {
    let (by, how): (Option<String>, Option<String>) = tx.query_row("SELECT started_by, started_how FROM runs WHERE id = ?1", [&run.id], |x| Ok((x.get(0)?, x.get(1)?))).map_err(sql)?;
    let ids: Vec<String> = blockers.iter().map(|b| b.id.clone()).collect();
    let press = Press::after(how.as_deref().and_then(How::of_word).unwrap_or_default(), by.as_deref().unwrap_or(HUMAN), &ids);
    let body = serde_json::to_value(press).map_err(|e| e.to_string())?;
    let keys: Vec<String> = blockers.iter().map(|b| b.key.clone()).collect();
    super::waits::add_after(tx, t, &run.agent_id, &t.project, &body, &keys, at)?;
    ops::log(tx, &t.key, DISPATCHER, "told", &format!("It waits for {}: Dispatcher goes on with it in its conversation once {} DONE.", ids.join(", "), if ids.len() == 1 { "that is" } else { "they are" }), None, at).map(drop)
}

#[cfg(test)]
#[path = "after_tests.rs"]
mod tests;
