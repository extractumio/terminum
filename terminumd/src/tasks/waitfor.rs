// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! wait_for_task (`terminum-dispatcher.md` AC-210): an agent's word that its task cannot go on before other tasks of
//! its project are DONE. They join its Blocked by (terminum-kanban.md AC-199) with the agent's reason, the task waits in
//! TODO, its session closes once the turn is over, and Dispatcher goes on with it in the same conversation when they are
//! DONE (`dispatcher/after.rs`). No human is asked unless one of them ends FAILED or archived.

use rusqlite::Transaction;
use serde_json::Value;

use super::attrs::{self, Link, LINKS_MOST, REASON_MOST};
use super::ops::{self, Task};
use super::tools::{arg, author, not_queued, own, Who};
use crate::dispatcher::after::{ended, is_are};
use crate::plans::rules;

/// The IDs `on` names, separated by commas or spaces, each once, in order.
fn named(on: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for id in on.split([',', ' ']).map(str::trim).filter(|s| !s.is_empty()) {
        if !ids.iter().any(|i| i.eq_ignore_ascii_case(id)) {
            ids.push(id.to_string());
        }
    }
    ids
}

/// Why a run cannot wait for tasks: it is not a run of Dispatcher's that can be gone on with.
fn not_dispatchers(id: &str) -> String {
    format!("{id} is not held by a run Dispatcher started, so nothing would go on with it by itself: link the tasks by execute_tool(\"link_task\", {{id, list: \"blocked_by\", task}}) and say so in your answer.")
}

/// This session's own task `args.id` waits for the tasks `args.on` names, `args.why` saying why; the task's ID.
pub(super) fn wait(tx: &Transaction, who: &Who, args: &Value, at: i64) -> Result<(String, Vec<String>), String> {
    let t = own(tx, who, arg(args, "id")?)?;
    not_queued(tx, &t)?;
    let id = t.id.as_str();
    let why = rules::clean(arg(args, "why")?);
    if why.chars().count() > REASON_MOST {
        return Err(format!("why: at most {REASON_MOST} characters."));
    }
    let ids = named(arg(args, "on")?);
    if ids.is_empty() {
        return Err("on: name the tasks it waits for, by their IDs.".into());
    }
    if ids.len() > LINKS_MOST {
        return Err(format!("on: at most {LINKS_MOST} tasks."));
    }
    let Some(run) = crate::dispatcher::runs::in_pane(tx, &who.key)?.filter(|r| !r.ad_hoc() && r.agent_session.is_some()) else { return Err(not_dispatchers(id)) };
    let others: Vec<&str> = run.open.iter().map(String::as_str).filter(|o| *o != id).collect();
    if !others.is_empty() {
        return Err(format!("This session also holds {}, not finished: go on with them first; {id} can wait once it is the last open task of this session, or ask a human by execute_tool(\"ask_human\", {{id, questions}}).", others.join(", ")));
    }
    let mut blockers: Vec<Task> = Vec::new();
    for other in &ids {
        let b = ops::get(tx, other)?;
        if b.key == t.key {
            return Err(format!("{id} cannot wait for itself."));
        }
        if let Some(how) = ended(&b.status, b.archived_at.is_some()) {
            return Err(format!("{} is {how}: it will not be DONE. If {id} cannot go on without it, ask a human by execute_tool(\"ask_human\", {{id, questions}}).", b.id));
        }
        if b.status == "DONE" {
            return Err(format!("{} is DONE already: bring in its work (merge the project's branch where it landed) and go on with {id}.", b.id));
        }
        blockers.push(b);
    }
    let me = author(tx, who)?;
    // Blocked by holds them with the reason: a list's own checks (its project's tasks, no task waiting for one that waits for it)
    for b in &blockers {
        attrs::link(tx, &me, &t.key, &Link { list: "blocked_by", task: &b.id, reason: Some(&why), remove: false }, at)?;
    }
    crate::dispatcher::after::keep(tx, &run, &me, id, &blockers, at)?;
    let them: Vec<&str> = blockers.iter().map(|b| b.id.as_str()).collect();
    let words = format!("{id} waits for {}: Dispatcher goes on with it in this conversation once {} {} DONE; nobody needs to be asked. End your turn now, with no more tool calls: Dispatcher closes this session.",
                        them.join(", "), if them.len() == 1 { "it" } else { "they" }, is_are(them.len()));
    Ok((words, vec![t.id]))
}

#[cfg(test)]
#[path = "waitfor_tests.rs"]
mod tests;
