// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum's tool catalog (terminum-agents.md AC-155): what search_tools indexes and
//! get_tool_info tells beside a tool's definition: its area, the words people use for what it
//! does, and an example call. A new tool gets its card here; a test refuses a tool without one.

use std::sync::OnceLock;

use serde_json::Value;

use super::mcp_search::{Doc, Index};

/// The areas, in the order search_tools lists them, each with what it holds.
pub const AREAS: [(&str, &str); 4] = [
    ("plan", "this session's steps, current step, ETA"),
    ("tasks", "list, read, create, change priority, parent or text, archive, take, move, ask the human, finish"),
    ("comments", "on a task"),
    ("files", "on a task: attach, get"),
];

pub struct Card {
    pub name: &'static str,
    pub area: &'static str,
    /// how people ask for it, beyond its description: search's second field
    pub keywords: &'static str,
    /// its arguments in a typical call, as JSON
    pub example: &'static str,
}

pub const CARDS: &[Card] = &[
    Card { name: "update_session_plan", area: "plan", keywords: "update my plan, report progress, mark a step completed, set the current step and ETA, checklist, todo list",
        example: r#"{"items": [{"text": "Read the code", "status": "completed"}, {"text": "Write the fix", "status": "in_progress"}], "doing": "Write the fix", "eta_minutes": 20}"# },
    Card { name: "get_session_plan", area: "plan", keywords: "show my plan, read the stored plan, what is left, current progress",
        example: "{}" },
    Card { name: "list_tasks", area: "tasks", keywords: "list tasks, see the board, open tasks, backlog, what to do next, filter by status or severity, find a task",
        example: r#"{"status": "TODO", "severity": "P1"}"# },
    Card { name: "get_task", area: "tasks", keywords: "read a task, open a ticket, task details, its comments and files, the assignment",
        example: r#"{"id": "TERM-12"}"# },
    Card { name: "create_task", area: "tasks", keywords: "create a task, add a new task, file a bug, open a ticket, new issue, feature request, tech debt, left to do, follow-up, subtask",
        example: r###"{"title": "Bug: the list loses its scroll on refresh", "severity": "P2", "text": "## Problem or goal\n...\n## Context\n...\n## Expected result\n...\n## Prerequisites\nNone."}"### },
    Card { name: "update_task", area: "tasks", keywords: "change priority, raise or lower severity, reprioritize, set parent, make a subtask, move under another task, regroup, take out of its parent, rename, retitle, edit the text, archive, delete, remove from the board, groom",
        example: r#"{"id": "TERM-12", "severity": "P1", "parent": "TERM-3", "reason": "it blocks TERM-3"}"# },
    Card { name: "take_task", area: "tasks", keywords: "take a task, claim, assign to me, pick up, start working on a task",
        example: r#"{"id": "TERM-12"}"# },
    Card { name: "move_task", area: "tasks", keywords: "move a task to a column, change its status, in progress, wait for a human, escalate, blocked, give up, failed, postpone, put back to todo",
        example: r#"{"id": "TERM-12", "to": "WAIT FOR HUMAN"}"# },
    Card { name: "ask_human", area: "tasks", keywords: "ask the human, ask the user, a question, need a decision, clarify, which option, wait for an answer, human in the loop, blocked on a choice",
        example: r#"{"id": "TERM-12", "questions": "1. Keep v1? Options: drop it, keep it a month. Recommended: keep it a month, two clients still call it."}"# },
    Card { name: "add_comment", area: "comments", keywords: "comment on a task, leave a note, reply, remark, say something on a ticket",
        example: r#"{"id": "TERM-12", "text": "The cause is in tasks/ops.rs; a fix needs the owner's choice."}"# },
    Card { name: "attach_file", area: "files", keywords: "attach a file, upload, add a screenshot, a log, a document, an artifact to a task",
        example: r#"{"task": "TERM-12", "path": "/Users/alex/project/report.md"}"# },
    Card { name: "get_file", area: "files", keywords: "get a task's file, download an attachment, open an attached document, its path",
        example: r#"{"task": "TERM-12", "name": "report.md"}"# },
    Card { name: "finish_task", area: "tasks", keywords: "finish my task, mark it done, complete, close, resolve, report the result, wrap up",
        example: r#"{"id": "TERM-12", "result": "Fixed in tasks/ops.rs; cargo test tasks passes; nothing left."}"# },
    Card { name: "dispatch_task", area: "tasks", keywords: "start a task on an agent, dispatch, Auto-Dispatcher's round, pick the next tasks, fill a free slot",
        example: r#"{"id": "TERM-12", "why": "RICE 8×2×0.8÷1 = 12.8: a small fix many sessions hit"}"# },
];

/// The cards a role finds: a worker the catalog's own, a manager those and its own (AC-174).
pub fn cards(manager: bool) -> impl Iterator<Item = &'static Card> {
    CARDS.iter().chain(super::mcp_manager::CARDS.iter().filter(move |_| manager))
}

/// The areas a role is told of.
pub fn areas(manager: bool) -> impl Iterator<Item = &'static (&'static str, &'static str)> {
    AREAS.iter().chain(super::mcp_manager::AREAS.iter().filter(move |_| manager))
}

pub fn card(name: &str) -> Option<&'static Card> {
    cards(true).find(|c| c.name == name)
}

/// Whether `name` is a manager's tool: a worker neither finds nor runs it.
pub fn manager_only(name: &str) -> bool {
    super::mcp_manager::CARDS.iter().any(|c| c.name == name)
}

/// Every tool's definition, a manager's too, built once.
pub fn definitions() -> &'static [Value] {
    static DEFS: OnceLock<Vec<Value>> = OnceLock::new();
    DEFS.get_or_init(|| {
        let mut all = super::mcp::catalog();
        all.extend(super::mcp_manager::tools());
        all
    })
}

pub fn definition(name: &str) -> Option<&'static Value> {
    definitions().iter().find(|d| d["name"] == name)
}

/// A role's search index: a tool's name, its keywords, its area and text, its arguments' names.
pub fn index(manager: bool) -> &'static Index {
    static INDEXES: [OnceLock<Index>; 2] = [OnceLock::new(), OnceLock::new()];
    INDEXES[manager as usize].get_or_init(|| {
        let docs: Vec<(String, String, &Card)> = cards(manager).filter_map(|c| {
            let d = definition(c.name)?;
            let args: Vec<&str> = d["inputSchema"]["properties"].as_object().map(|p| p.keys().map(String::as_str).collect()).unwrap_or_default();
            Some((format!("{} {}", d["title"].as_str().unwrap_or(""), d["description"].as_str().unwrap_or("")), args.join(" "), c))
        }).collect();
        Index::new(&docs.iter().map(|(text, args, c)| Doc { name: c.name, fields: vec![(c.keywords, 2.0), (c.area, 1.0), (text, 1.0), (args, 1.0)] }).collect::<Vec<_>>())
    })
}
