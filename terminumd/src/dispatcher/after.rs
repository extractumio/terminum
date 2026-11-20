// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A run that waits for other tasks (`terminum-dispatcher.md` AC-210): its agent said so by wait_for_task; the session
//! closes as after a question (AC-157), and a kept press (AC-158) goes on in the same conversation once every task it
//! waits for is DONE, with fixed words and their IDs alone; one of them FAILED, archived or gone asks a human.

use rusqlite::Transaction;

use super::api::Press;
use super::runs::Run;
use crate::tasks::ops::{self, rows, Task};
use crate::tasks::store::DISPATCHER;

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

/// How task `t` ended without being DONE, if it did: it never will be.
pub(crate) fn ended(status: &str, archived: bool) -> Option<&'static str> {
    if archived { Some("archived") } else if status == "FAILED" { Some("FAILED") } else { None }
}

/// "is" or "are", for `n` tasks.
pub(crate) fn is_are(n: usize) -> &'static str {
    if n == 1 { "is" } else { "are" }
}

/// Where the tasks of store keys `keys`, which task `id` waits for, stand: one narrow read a walk, of status alone.
pub(super) fn blockers(tx: &Transaction, id: &str, keys: &[String]) -> Result<Blockers, String> {
    let found: Vec<(String, String, bool)> = rows(tx, "SELECT slug, status, archived_at IS NOT NULL FROM tasks WHERE id IN (SELECT value FROM json_each(?1))",
        [serde_json::json!(keys).to_string()], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    if found.len() < keys.len() {
        return Ok(Blockers::Gone(gone(id, "a task that was deleted", "deleted")));
    }
    if let Some((slug, how)) = found.iter().find_map(|(slug, status, archived)| ended(status, *archived).map(|how| (slug, how))) {
        return Ok(Blockers::Gone(gone(id, slug, how)));
    }
    Ok(if found.iter().all(|(_, status, _)| status == "DONE") { Blockers::Done } else { Blockers::Open })
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
    format!("[terminum Dispatcher] {id}: {}, which it waited for, {} DONE, and Dispatcher resumed this conversation. \
        Go on with {id}: bring in their work (merge the project's branch where they landed), run your checks again, then finish it as you were asked.", after.join(", "), is_are(after.len()))
}

/// The words of a session closed while its task waits for other tasks.
pub const CLOSED: &str = "Session closed: it waits for other tasks. Dispatcher resumes this conversation once they are DONE.";

/// Task `id`, which `run` holds, now waits for `blockers` (tasks of its project, none DONE, FAILED or archived), moved
/// to TODO by `author`: its press is kept, as who started the run and how (AC-183), to go on once they are DONE, and its
/// session closes as after a question. The wait remembers TODO, the column a move away from calls it off (AC-158).
pub(crate) fn keep(tx: &Transaction, run: &Run, author: &str, id: &str, blockers: &[Task], at: i64) -> Result<(), String> {
    ops::move_to(tx, author, id, "TODO", at)?;
    super::runs::change(tx, &run.id, |r| r.asked = true)?;
    let (how, by) = super::runs::started(tx, &run.id)?;
    let ids: Vec<String> = blockers.iter().map(|b| b.id.clone()).collect();
    let body = serde_json::to_value(Press::after(how, &by, &ids)).map_err(|e| e.to_string())?;
    // store keys, which stay when a task moves to another project and its ID changes
    let keys: Vec<String> = blockers.iter().map(|b| b.key.clone()).collect();
    let t = ops::get(tx, id)?;
    super::waits::add_after(tx, &t, &run.agent_id, &t.project, &body, &keys, at)?;
    ops::log(tx, &t.key, DISPATCHER, "told", &format!("It waits for {}: Dispatcher goes on with it in its conversation once {} DONE.", ids.join(", "), if ids.len() == 1 { "that is" } else { "they are" }), None, at).map(drop)
}

#[cfg(test)]
#[path = "after_tests.rs"]
mod tests;
