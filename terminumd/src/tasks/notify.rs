// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! A new comment on a task reaches the agent working on it (terminum-dispatcher.md AC-209): a notice in fixed words
//! with the task's ID and who wrote it, never the comment's words, waits for the agent's next hook that carries
//! context (tell.rs `Lines`); the agent reads the comment itself by get_task. Nothing is typed into its pane.
//! What became of each notice is said on its task, and the task's window marks each comment by it.

use std::collections::{HashMap, HashSet};

use rusqlite::{OptionalExtension, Transaction};
use serde_json::{json, Value};

use super::ops;
use super::store::DISPATCHER;
use super::tell::Line;
use crate::app::Shared;

/// Why a notice did not reach its session.
pub const ENDED: &str = "its agent's session ended before its next tool call";
pub const CROWDED: &str = "more than 20 lines waited for its agent";
pub const RUN_ENDED: &str = "its agent was ended before its next tool call";
pub const NO_HOOKS: &str = "its agent's hooks do not reach terminum: run terminum hooks install, and see terminum hooks for a Codex under a shared app-server";

/// The most of a name a notice says, and the characters it may hold: else it names the author's kind alone.
const NAME_MOST: usize = 32;

/// Who wrote a comment, by its author's kind: as a notice and a fresh start say it.
pub fn kind_words(kind: &str, manager: bool) -> &'static str {
    if manager { "the project's manager" } else if kind == "human" { "a human" } else { "an agent" }
}

/// Who wrote a comment, in a notice's words: the author's kind, and its name when it is a plain one.
pub fn by_words(kind: &str, manager: bool, name: &str) -> String {
    let who = kind_words(kind, manager);
    let plain = !name.is_empty() && name.chars().count() <= NAME_MOST
        && name.chars().all(|c| c.is_alphanumeric() || " ._-·".contains(c)) && name.trim() == name;
    if plain { format!("{who} \"{name}\"") } else { who.to_string() }
}

/// The notice of a task's new comments: fixed words, the ID, who wrote them, how many (AC-209).
pub fn words(task: &str, comments: &[(String, String)]) -> String {
    let mut by: Vec<&str> = Vec::new();
    for (_, b) in comments {
        if !by.contains(&b.as_str()) {
            by.push(b);
        }
    }
    let others = by.len() > 3;
    by.truncate(3);
    let named = match (by.as_slice(), others) {
        ([one], _) => one.to_string(),
        (all, true) => format!("{} and others", all.join(", ")),
        ([init @ .., last], false) => format!("{} and {last}", init.join(", ")),
        ([], _) => String::new(),
    };
    let get = format!("execute_tool(\"get_task\", {{\"id\": \"{task}\"}})");
    match comments.len() {
        1 => format!("[terminum] {task} has a new comment by {named}: read it with {get} and adjust your work if it changes what the task asks."),
        n => format!("[terminum] {task} has {n} new comments, by {named}: read them with {get} and adjust your work if they change what the task asks."),
    }
}

/// What became of a notice, said on its task: told (`why` none), else not and why; the comments it told of in its data.
pub fn said(tx: &Transaction, line: &Line, why: Option<&str>, at: i64) -> Result<String, String> {
    let key = ops::key_of(tx, &line.task)?;
    let of = match line.comments.len() {
        1 => "the new comment".to_string(),
        n => format!("{n} new comments"),
    };
    let text = match why {
        None => format!("Told the session of {of}"),
        Some(why) => format!("Not told the session of {of}: {why}"),
    };
    let ids: Vec<&str> = line.comments.iter().map(|(e, _)| e.as_str()).collect();
    ops::log(tx, &key, DISPATCHER, "told", &text, Some(&json!({"comments": ids, "not": why.is_some()})), at)
}

/// Notices that never reached their session are said so on their tasks, in one transaction; Dispatcher's lines
/// among them are left.
pub fn lost(app: &Shared, lines: Vec<Line>, why: &'static str) {
    let lines: Vec<Line> = lines.into_iter().filter(Line::is_notice).collect();
    if lines.is_empty() {
        return;
    }
    let a = app.clone();
    tokio::task::spawn_blocking(move || {
        let at = ops::now();
        let said = a.tasks.with(|tx| Ok(lines.iter().filter_map(|l| match said(tx, l, Some(why), at) {
            Ok(_) => Some(l.task.clone()),
            Err(e) => { tracing::warn!(event = "tasks.notice", task = %l.task, error = %e); None }
        }).collect::<HashSet<String>>()));
        said.unwrap_or_default().iter().for_each(|task| super::api::history_heard(&a, task));
    });
}

/// A comment's task's ID as it stands (`task` may be an old one), its session and who wrote it, when a notice may
/// go: a comment by no system user (AC-209).
fn of_comment(tx: &Transaction, task: &str, event: &str) -> Result<Option<(String, String, String)>, String> {
    let key = ops::key_of(tx, task)?;
    let found: Option<(String, Option<String>, String, String, bool)> = tx.query_row(
        &format!("SELECT t.slug, t.session_key, u.kind, u.name, {} FROM events e
                  JOIN users u ON u.id = e.author JOIN tasks t ON t.id = e.task WHERE e.id = ?1 AND e.task = ?2 AND e.kind = 'comment'", super::comments::BY_MANAGER),
        (event, &key), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))).optional().map_err(ops::sql)?;
    Ok(found.and_then(|(id, session, kind, name, manager)| (kind != "system").then(|| session.map(|s| (id, s, by_words(&kind, manager, &name)))).flatten()))
}

/// What a new comment's notice comes to (AC-209), its session read from the list as it stands.
#[derive(Debug, PartialEq)]
pub enum Goes {
    /// no linked session open with an agent, or the session's own agent wrote it: nothing is said
    Nowhere,
    /// its agent's hooks never spoke: it cannot be told
    NoHooks,
    /// it waits for the agent's next hook that carries context
    Waits,
}

/// Whether the session's row in the list shows its agent running on a server that is read.
pub fn agent_runs(row: Option<&Value>) -> bool {
    row.is_some_and(|r| r["offline"] != true && r["agent"].as_str().is_some_and(|a| !a.is_empty()))
}

/// Where a comment's notice goes: `runs` its session's agent (`agent_runs`), `from` the pane whose agent wrote it.
pub fn goes(key: &str, runs: bool, hooks: bool, from: Option<&str>) -> Goes {
    match (from == Some(key), runs, hooks) {
        (true, _, _) | (_, false, _) => Goes::Nowhere,
        (_, true, false) => Goes::NoHooks,
        (_, true, true) => Goes::Waits,
    }
}

/// A comment was added to `task`, or a human's changed its text: its session hears of it at its agent's next tool
/// call (AC-209). `from`: the pane whose agent wrote it, never told of its own.
pub fn comment_added(app: &Shared, task: &str, event: &str, from: Option<&str>) {
    let (a, task, event, from) = (app.clone(), task.to_string(), event.to_string(), from.map(str::to_string));
    tokio::task::spawn_blocking(move || {
        let Ok(Some((task, key, by))) = a.tasks.with(|tx| of_comment(tx, &task, &event)) else { return };
        let runs = agent_runs(crate::page::marks::find_item(&a.layout.read(), &key));
        match goes(&key, runs, a.hooks.has(&key), from.as_deref()) {
            Goes::Nowhere => {}
            Goes::NoHooks => lost(&a, vec![Line { task, text: String::new(), comments: vec![(event, by)] }], NO_HOOKS),
            Goes::Waits => {
                let dropped = a.task_lines.push_comment(&key, &task, &event, by);
                lost(&a, dropped.into_iter().collect(), CROWDED);
            }
        }
    });
}

/// Each comment of a task's history marked by what became of its notice (AC-209): "told", "not", or "waiting"
/// while its notice is in `waiting`; a comment with none has no mark.
pub fn marks(history: &mut Value, waiting: &HashSet<String>) {
    let Some(rows) = history.as_array_mut() else { return };
    let mut outcome: HashMap<String, &'static str> = HashMap::new();
    for r in rows.iter() {
        for id in r["comments"].as_array().into_iter().flatten().filter_map(Value::as_str) {
            outcome.insert(id.to_string(), if r["not"] == true { "not" } else { "told" });
        }
    }
    for r in rows.iter_mut().filter(|r| r["kind"] == "comment") {
        let id = r["id"].as_str().unwrap_or_default();
        // a comment changed since its notice waits again: what waits now counts
        if let Some(mark) = if waiting.contains(id) { Some("waiting") } else { outcome.get(id).copied() } {
            r["told"] = json!(mark);
        }
    }
}

#[cfg(test)]
#[path = "notify_tests.rs"]
mod tests;
