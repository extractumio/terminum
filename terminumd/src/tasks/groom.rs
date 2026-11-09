// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! An agent grooms its project's board with update_task (terminum-kanban.md AC-154): the severity
//! and parent of any task it reaches, a human's with a reason; the title and text of an agents'
//! task no other session holds; such a task off the board, its reason said. Each change is in the
//! task's history, its author the agent, as a human's is.

use rusqlite::Transaction;
use serde_json::Value;

use super::family;
use super::ops::{self, Edit, Task};
use super::tools::{arg, author, title_of, visible, Who};
use crate::plans::rules;

/// What `parent` says to take a task out of its parent.
const NO_PARENT: &str = "none";

/// An agents' task: made by an agent, its words never changed by a human since, and no other
/// session's. Its title and text are agents' to change, and it can leave the board.
fn agents(tx: &Transaction, who: &Who, t: &Task) -> Result<(), String> {
    if !t.by_agent || ops::worded_by_human(tx, &t.key)? {
        return Err(format!("{} holds a human's words: they decide its title, text and archive. Comment instead.", t.id));
    }
    match &t.session {
        Some(s) if s != &who.key && t.status != "DONE" => Err(format!("{} belongs to session {}: comment instead.", t.id, ops::pane_of(s))),
        _ => Ok(()),
    }
}

/// update_task: what it says, and the tasks whose cards change (a parent left or joined too).
pub(super) fn update(tx: &Transaction, who: &Who, args: &Value, at: i64) -> Result<(String, Vec<String>), String> {
    let id = arg(args, "id")?;
    let t = visible(tx, who, id)?;
    let field = |k: &str| args[k].as_str().filter(|s| !s.is_empty());
    let (title, text, severity, parent) = (field("title"), field("text"), field("severity"), field("parent"));
    let reason = field("reason").map(rules::clean);
    if field("archive").is_some() {
        if title.or(text).or(severity).or(parent).is_some() {
            return Err("archive goes alone, with its reason.".into());
        }
        let reason = reason.ok_or("Give the reason it leaves the board.")?;
        return archive(tx, who, &t, &reason, at);
    }
    if title.or(text).or(severity).or(parent).is_none() {
        return Err("Give severity, parent, title, text or archive.".into());
    }
    if title.or(text).is_some() {
        agents(tx, who, &t)?;
    }
    // a human's task changes with its why in its history
    if !t.by_agent && reason.is_none() {
        return Err(format!("{id} is a human's: give the reason for the change."));
    }
    let title = title.map(title_of).transpose()?;
    let text = match text {
        Some(s) => {
            let s = rules::clean(s);
            super::template::check(&s, &super::template::of_project(tx, &t.project)?)?;
            Some(s)
        }
        None => None,
    };
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
    let kids: Vec<(String, String)> = ops::rows(tx, "SELECT slug, status FROM tasks WHERE parent = ?1 AND archived_at IS NULL", [&t.key], |r| Ok((r.get(0)?, r.get(1)?)))?;
    if let Some((open, _)) = kids.iter().find(|(_, status)| status != "DONE") {
        return Err(format!("{id} has an open subtask, {open}: finish it, archive it or take it out first."));
    }
    let me = author(tx, who)?;
    ops::comment(tx, &me, id, &format!("Archived: {reason}"), at)?;
    let mut cards: Vec<String> = kids.into_iter().map(|(kid, _)| kid).collect();
    cards.push(id.clone());
    for task in &cards {
        ops::archive(tx, &me, task, true, at)?;
    }
    cards.extend(t.parent.clone());
    Ok((format!("{id} is archived: a human can bring it back from the Archived list."), cards))
}

#[cfg(test)]
#[path = "groom_tests.rs"]
mod tests;
