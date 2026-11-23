// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! finish_task (`terminum-dispatcher.md` AC-123, AC-166): an agent's result in its sections makes its task DONE, or,
//! a report for a person to read, makes it wait for its human (AC-264); a result without them, or of work not done,
//! stays a comment.

use rusqlite::Transaction;
use serde_json::Value;

use super::closeout;
use super::tools::{arg, movable, Who};
use super::{ops::{self, Task}, report};
use crate::plans::rules;

/// `args.id`'s result `args.result`, written by `me`: what the agent is told, and the tasks it changed.
pub(super) fn finish(tx: &Transaction, who: &Who, args: &Value, me: &str, at: i64) -> Result<(String, Vec<String>), String> {
    let id = arg(args, "id")?;
    let t = movable(tx, who, id)?;
    if let Some(why) = crate::integrate::agent::finish_refused(tx, &t.key)? {
        return Err(why);
    }
    super::integration_queue::stays_queued(tx, &t)?;
    after_its_subtasks(tx, &t)?;
    let result = rules::clean(arg(args, "result")?);
    let review = crate::supervisor::review::of_task(tx, &t.key)?;
    if let Some(project) = &review {
        crate::supervisor::review::require(tx, &who.key, project, &t.id)?;
        if !crate::supervisor::review::complete(tx, &t, project, me, &result, at)? {
            return Ok(("Use the nine Supervisor review sections in the order shown in the task prompt.".into(), Vec::new()));
        }
    } else {
        ops::comment(tx, me, id, &result, at)?;
        // A malformed result stays a comment and leaves the task in place (AC-123).
        let form = closeout::form_of(tx, &t.key)?;
        let lacks = closeout::missing(&result, form);
        if !lacks.is_empty() {
            return Ok((closeout::missing_words(id, &t.status, &lacks), vec![id.to_string()]));
        }
        if form == closeout::SECTIONS.as_slice() && closeout::not_done(&result) {
            return Ok((closeout::not_done_words(id, &t.status), vec![id.to_string()]));
        }
    }
    let mut changed = vec![id.to_string()];
    let mut said = if report::wanted(tx, &t.key, args)? {
        changed.extend(report::wait(tx, me, &t, at)?);
        vec![format!("{id} waits for the human to read your report.")]
    } else {
        ops::move_to(tx, me, id, "DONE", at)?;
        vec![format!("{id} is DONE, with your result.")]
    };
    crate::dispatcher::runs::finished(tx, &t.key)?;
    if review.is_some() { return Ok((said.join(" "), changed)); }
    // what it leaves is on the board by terminum's word, and what it names and is not there is said (AC-166)
    said.extend(closeout::made_line(tx, &t)?);
    let unknown = closeout::unknown_ids(tx, &result)?;
    if !unknown.is_empty() {
        said.push(closeout::unknown_words(&unknown));
    }
    let untracked = closeout::untracked(tx, &result)?;
    if !untracked.is_empty() {
        said.push(closeout::untracked_words(&untracked));
    }
    Ok((said.join(" "), changed))
}

/// A main task ends last: refused while a subtask its run took is not DONE (terminum-dispatcher.md AC-185).
pub(super) fn after_its_subtasks(tx: &Transaction, t: &Task) -> Result<(), String> {
    let open = crate::dispatcher::runs::subtasks_open(tx, &t.key)?;
    if open.is_empty() {
        return Ok(());
    }
    Err(format!("{0} is a main task: its subtasks of this run are not DONE: {1}. Finish them first; when a cause that can be removed {2} holds one, ask by execute_tool(\"ask_human\", {{id, questions}}); when one cannot be done at all, move {0} to FAILED with a comment saying why.", t.id, open.join(", "), super::closeout::removable!()))
}
