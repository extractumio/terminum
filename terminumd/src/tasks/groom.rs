// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! An agent grooms its project's board with update_task (terminum-kanban.md AC-154): the severity
//! and parent of any task it reaches, a human's with a reason; the title and text of an agents'
//! task no human's RUN holds in another session; such a task off the board, its reason said. Each change is in the
//! task's history, its author the agent, as a human's is.

use rusqlite::Transaction;
use serde_json::Value;

use super::family;
use super::ops::{self, Edit, Task};
use super::tools::{arg, author, not_run_elsewhere, visible, words_of, Who};
use crate::plans::rules;

/// What `parent` says to take a task out of its parent.
const NO_PARENT: &str = "none";

/// An agents' task: made by an agent, its words never changed by a human since, and not what a
/// human's RUN holds in another session. Its title and text are agents' to change, and it can leave the board.
fn agents(tx: &Transaction, who: &Who, t: &Task) -> Result<(), String> {
    if !t.by_agent || ops::worded_by_human(tx, &t.key)? {
        return Err(format!("{} holds a human's words: they decide its title, text and archive. Comment instead.", t.id));
    }
    not_run_elsewhere(tx, who, t)
}

/// update_task: what it says, and the tasks whose cards change (a parent left or joined too).
pub(super) fn update(tx: &Transaction, who: &Who, args: &Value, at: i64) -> Result<(String, Vec<String>), String> {
    let id = arg(args, "id")?;
    let t = visible(tx, who, id)?;
    let field = |k: &str| args[k].as_str().filter(|s| !s.is_empty());
    let (title, text, severity, parent) = (field("title"), field("text"), field("severity"), field("parent"));
    let reason = field("reason").map(rules::clean);
    let change = title.or(text).or(severity).or(parent);
    if field("archive").is_some() {
        if change.is_some() {
            return Err("archive goes alone, with its reason.".into());
        }
        let reason = reason.ok_or("Give the reason it leaves the board.")?;
        return archive(tx, who, &t, &reason, at);
    }
    if change.is_none() {
        return Err("Give severity, parent, title, text or archive.".into());
    }
    // a human's task changes with its why in its history
    if !t.by_agent && reason.is_none() {
        return Err(format!("{id} is a human's: give the reason for the change."));
    }
    if title.is_some() || text.is_some() {
        agents(tx, who, &t)?;
    }
    let mut faults = Vec::new();
    let (title, text) = words_of(tx, &t.project, title, text, &mut faults);
    if !faults.is_empty() {
        return Err(faults.join("\n"));
    }
    let me = author(tx, who)?;
    ops::edit(tx, &me, &t.id, Edit { title, text, severity: severity.map(str::to_string), owner: None }, at)?;
    let mut cards = vec![t.id.clone()];
    if let Some(p) = parent {
        let to = if p == NO_PARENT { None } else { Some(visible(tx, who, p)?.id) };
        family::set_parent(tx, &me, &t.id, to.as_deref(), at)?;
        cards.extend(t.parent.into_iter().chain(to));
    }
    if let Some(r) = reason {
        ops::comment(tx, &me, &t.id, &r, at)?;
    }
    Ok((format!("{id} is changed."), cards))
}

/// An agents' task off the board, its reason a comment; its DONE subtasks go with it, as when it
/// archives itself (AC-118); a human brings it back from the Archived list.
fn archive(tx: &Transaction, who: &Who, t: &Task, reason: &str, at: i64) -> Result<(String, Vec<String>), String> {
    let id = &t.id;
    if t.archived_at.is_some() {
        return Ok((format!("{id} is archived already."), Vec::new()));
    }
    agents(tx, who, t)?;
    if crate::dispatcher::runs::holding(tx, &t.key)?.is_some() {
        return Err(format!("{id} is what a human ran: finish it, fail it, or ask."));
    }
    let open: Vec<String> = ops::rows(tx, "SELECT slug FROM tasks WHERE parent = ?1 AND status <> 'DONE' AND archived_at IS NULL LIMIT 1", [&t.key], |r| r.get(0))?;
    if let Some(open) = open.first() {
        return Err(format!("{id} has an open subtask, {open}: finish it, archive it or take it out first."));
    }
    let me = author(tx, who)?;
    ops::comment(tx, &me, id, &format!("Archived: {reason}"), at)?;
    let mut cards = super::archiving::with_done_subtasks(tx, &me, &t.key, id, at)?;
    cards.extend(t.parent.clone());
    Ok((format!("{id} is archived: a human can bring it back from the Archived list."), cards))
}

#[cfg(test)]
#[path = "groom_tests.rs"]
mod tests;
