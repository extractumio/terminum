// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! finish_task (`terminum-dispatcher.md` AC-123, AC-166): an agent's result in its sections makes its task DONE, or,
//! a report for a person to read, makes it wait for its human (AC-264); a result without them, or of work not done,
//! stays a comment.

use rusqlite::Transaction;
use serde_json::Value;

use super::closeout;
use super::tools::{after_its_subtasks, arg, movable, Who};
use super::{ops, report};
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
    ops::comment(tx, me, id, &result, at)?;
    // a result is there only in its sections: else its words stay a comment, the task as it was (AC-123)
    let form = closeout::form_of(tx, &t.key)?;
    let lacks = closeout::missing(&result, form);
    if !lacks.is_empty() {
        return Ok((closeout::missing_words(id, &t.status, &lacks), vec![id.to_string()]));
    }
    // work its own Outcome says is not done is no DONE: it waits for a human or fails (AC-123, AC-157)
    if form == closeout::SECTIONS.as_slice() && closeout::not_done(&result) {
        return Ok((closeout::not_done_words(id, &t.status), vec![id.to_string()]));
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
