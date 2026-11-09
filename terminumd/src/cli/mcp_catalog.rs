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
    ("tasks", "list, read, create, change priority, parent or text, archive, take, move, finish"),
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
    Card { name: "add_comment", area: "comments", keywords: "comment on a task, leave a note, reply, remark, say something on a ticket",
        example: r#"{"id": "TERM-12", "text": "The cause is in tasks/ops.rs; a fix needs the owner's choice."}"# },
    Card { name: "attach_file", area: "files", keywords: "attach a file, upload, add a screenshot, a log, a document, an artifact to a task",
        example: r#"{"task": "TERM-12", "path": "/Users/alex/project/report.md"}"# },
    Card { name: "get_file", area: "files", keywords: "get a task's file, download an attachment, open an attached document, its path",
        example: r#"{"task": "TERM-12", "name": "report.md"}"# },
    Card { name: "finish_task", area: "tasks", keywords: "finish my task, mark it done, complete, close, resolve, report the result, wrap up",
        example: r#"{"id": "TERM-12", "result": "Fixed in tasks/ops.rs; cargo test tasks passes; nothing left."}"# },
];

pub fn card(name: &str) -> Option<&'static Card> {
    CARDS.iter().find(|c| c.name == name)
}

/// Every tool's definition, built once.
pub fn definitions() -> &'static [Value] {
    static DEFS: OnceLock<Vec<Value>> = OnceLock::new();
    DEFS.get_or_init(super::mcp::catalog)
}

pub fn definition(name: &str) -> Option<&'static Value> {
    definitions().iter().find(|d| d["name"] == name)
}

/// The search index: a tool's name, its keywords, its area and text, its arguments' names.
pub fn index() -> &'static Index {
    static INDEX: OnceLock<Index> = OnceLock::new();
    INDEX.get_or_init(|| {
        let docs: Vec<(String, String, &Card)> = definitions().iter().filter_map(|d| {
            let c = card(d["name"].as_str()?)?;
            let args: Vec<&str> = d["inputSchema"]["properties"].as_object().map(|p| p.keys().map(String::as_str).collect()).unwrap_or_default();
            Some((format!("{} {}", d["title"].as_str().unwrap_or(""), d["description"].as_str().unwrap_or("")), args.join(" "), c))
        }).collect();
        Index::new(&docs.iter().map(|(text, args, c)| Doc { name: c.name, fields: vec![(c.keywords, 2.0), (c.area, 1.0), (text, 1.0), (args, 1.0)] }).collect::<Vec<_>>())
    })
}
