// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! finish_task (`terminum-dispatcher.md` AC-123, AC-166): an agent's result in its sections makes its task DONE, a
//! routine's result a report in the human's Inbox too (terminum-reports.md AC-272); a result without them, or of work
//! not done, stays a comment.

use rusqlite::Transaction;
use serde_json::Value;

use super::closeout;
use super::tools::{arg, movable, Who};
use super::ops::{self, Task};
use crate::plans::rules;

/// `args.id`'s result `args.result`, written by `me`: what the agent is told, and the tasks it changed.
pub(super) fn finish(tx: &Transaction, who: &Who, args: &Value, me: &str, at: i64) -> Result<(String, Vec<String>), String> {
    let id = arg(args, "id")?;
    if args.get("report").is_some_and(|r| !r.is_null()) {
        return Err(GONE_REPORT.into());
    }
    let t = movable(tx, who, id)?;
    if let Some(why) = crate::integrate::agent::finish_refused(tx, &t.key)? {
        return Err(why);
    }
    super::integration_queue::stays_queued(tx, &t)?;
    after_its_subtasks(tx, &t)?;
    let raw = arg(args, "result")?;
    let result = rules::clean(raw);
    let review = crate::supervisor::review::of_task(tx, &t.key)?;
    if let Some((project, audit)) = &review {
        crate::supervisor::review::require(tx, &who.key, project, &t.id)?;
        if *audit {
            let (_, run, _) = crate::supervisor::audit::marked(tx, &who.key)?
                .ok_or("The quarterly audit is inactive.")?;
            crate::supervisor::audit::check_start(tx, &run)?;
            crate::supervisor::audit::complete(tx, project, &t, me, raw, at)?;
        } else if !crate::supervisor::review::complete(tx, &t, project, me, &result, at)? {
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
    let changed = vec![id.to_string()];
    ops::move_to(tx, me, id, "DONE", at)?;
    let mut said = vec![match crate::reports::sources::routine_result(tx, &t, &result, me, at)? {
        Some(report) => format!("{id} is DONE; its result went to the human's Inbox as {report}."),
        None => format!("{id} is DONE, with your result."),
    }];
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

/// What finish_task's old report argument is refused with: the report goes by its own tool (terminum-reports.md AC-266).
pub(crate) const GONE_REPORT: &str = "finish_task takes no report: give the report by execute_tool(\"create_report\", {task, title, summary, body}), then call finish_task again without it.";

/// A main task ends last: refused while a subtask its run took is not DONE (terminum-dispatcher.md AC-185).
pub(super) fn after_its_subtasks(tx: &Transaction, t: &Task) -> Result<(), String> {
    let open = crate::dispatcher::runs::subtasks_open(tx, &t.key)?;
    if open.is_empty() {
        return Ok(());
    }
    Err(format!("{0} is a main task: its subtasks of this run are not DONE: {1}. Finish them first; when a cause that can be removed {2} holds one, ask by execute_tool(\"ask_human\", {{id, questions}}); when one cannot be done at all, move {0} to FAILED with a comment saying why.", t.id, open.join(", "), super::closeout::removable!()))
}
