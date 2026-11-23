// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! terminum's tool catalog (terminum-agents.md AC-155): what search_tools indexes and
//! get_tool_info tells beside a tool's definition: its area, the words people use for what it
//! does, and an example call. A new tool gets its card here; a test refuses a tool without one.

use std::sync::OnceLock;

use serde_json::Value;

use crate::roles::{Role, supervisor::START_ARGS};
use crate::supervisor::Level;

use super::mcp_search::{Doc, Index};

/// The areas, in the order search_tools lists them, each with what it holds.
pub const AREAS: [(&str, &str); 5] = [
    ("plan", "this session's steps, current step, ETA"),
    ("tasks", "list, read, create, change priority, parent or text, archive, take, move, ask the human, finish"),
    ("comments", "on a task"),
    ("files", "on a task: attach, get"),
    ("supervisor", "this project’s case, signals, metrics and finding"),
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
    Card { name: "update_task", area: "tasks", keywords: "change priority, raise or lower severity, reprioritize, set parent, make a subtask, move under another task, regroup, take out of its parent, rename, retitle, edit the text, set the type (feature, bug, debt, story), external link to Jira or Linear, target release, archive, delete, remove from the board, groom",
        example: r#"{"id": "TERM-12", "severity": "P1", "parent": "TERM-3", "reason": "it blocks TERM-3"}"# },
    Card { name: "link_task", area: "tasks", keywords: "blocked by, depends on, waits for, blocker, dependency, prerequisite, follow-up, next task, link tasks, unlink, relation",
        example: r#"{"id": "TERM-12", "list": "blocked_by", "task": "TERM-9", "reason": "needs the new store column first"}"# },
    Card { name: "take_task", area: "tasks", keywords: "take a task, claim, assign to me, pick up, start working on a task",
        example: r#"{"id": "TERM-12"}"# },
    Card { name: "move_task", area: "tasks", keywords: "move a task to a column, change its status, in progress, wait for a human, escalate, blocked, give up, failed, postpone, put back to todo",
        example: r#"{"id": "TERM-12", "to": "WAIT FOR HUMAN"}"# },
    Card { name: "ask_human", area: "tasks", keywords: "ask the human, ask the user, a question, need a human, need a decision, clarify, which option, wait for an answer, human in the loop, blocked on a choice",
        example: r#"{"id": "TERM-12", "questions": "Why you are needed: dropping API v1 breaks two clients, and only you know if they may break.\nRecommended:\n1. Keep v1 a month: they move by then. Other options: drop it now.\nDetails: TERM-12 removes API v1; v2 is done on a branch. Two clients still call v1; dropping it breaks them today.", "kind": "strategic", "options": [{"id": "keep", "label": "Keep v1 a month"}, {"id": "drop", "label": "Drop v1 now"}], "recommended": "keep"}"# },
    Card { name: "wait_for_task", area: "tasks", keywords: "wait until a blocker is done, blocked by a ticket, depends on, dependency, needs its change first, prerequisite not done, resume me when it lands, hold until",
        example: r#"{"id": "TERM-12", "on": "TERM-9", "why": "TERM-12 reads the store column TERM-9 adds; it goes on once TERM-9 lands."}"# },
    Card { name: "add_comment", area: "comments", keywords: "comment on a task, leave a note, reply, remark, say something on a ticket",
        example: r#"{"id": "TERM-12", "text": "The cause is in tasks/ops.rs; a fix needs the owner's choice."}"# },
    Card { name: "attach_file", area: "files", keywords: "attach a file, upload, add a screenshot, a log, a document, an artifact to a task",
        example: r#"{"task": "TERM-12", "path": "/Users/alex/project/report.md"}"# },
    Card { name: "get_file", area: "files", keywords: "get a task's file, download an attachment, open an attached document, its path",
        example: r#"{"task": "TERM-12", "name": "report.md"}"# },
    Card { name: "finish_task", area: "tasks", keywords: "finish my task, mark it done, complete, close, resolve, report the result, wrap up",
        example: r#"{"id": "TERM-12", "result": "Fixed in tasks/ops.rs; cargo test tasks passes; nothing left."}"# },
    Card { name: "integrate", area: "tasks", keywords: "land, merge my work, integrate, ship, publish to main, push, queue for integration, make land, deliver the commit",
        example: r#"{"id": "TERM-12", "result": "Outcome: Done ..."}"# },
    Card { name: "withdraw_integration", area: "tasks", keywords: "take out of the integration queue, cancel the landing, withdraw, unqueue, stop integrating",
        example: r#"{"id": "TERM-12"}"# },
    Card { name: "integration_status", area: "tasks", keywords: "integration queue, landing queue, my place in the queue, batch, what is being checked, merge queue status",
        example: r#"{"id": "TERM-12"}"# },
    Card { name: "integration_return", area: "tasks", keywords: "red batch, return the task that broke the check, blame, culprit, failing commit, analysis",
        example: r#"{"task": "TERM-12", "analysis": "Failing step: cargo ...\nEvidence: ...\nCommit and lines: ...\nWhat to fix: ..."}"# },
    Card { name: "integration_recheck", area: "tasks", keywords: "check the batch again, rebuild the candidate, flaky test, retry the check",
        example: r#"{"why": "a port was held by a stale process: ended it"}"# },
    Card { name: "integration_add_fix", area: "tasks", keywords: "fix the target, main is red, add my commit to the candidate",
        example: r#"{"commit": "9c1d0aa"}"# },
    Card { name: "dispatch_task", area: "tasks", keywords: "start a task on an agent, dispatch, Auto-Dispatcher's round, pick the next tasks, fill a free slot",
        example: r#"{"id": "TERM-12", "why": "RICE 8×2×0.8÷1 = 12.8; tasks/ops.rs moves; a small fix many sessions hit"}"# },
    Card { name: crate::supervisor::design_gates::BRIEF, area: "tasks", keywords: "request design first architecture review migration rollback brief",
        example: r#"{"id":"API-12","design_note":"Version the settings document.","migration":"Read both versions.","rollback":"Keep writing the old version until verification.","paths":"store/settings.json"}"# },
    Card { name: crate::supervisor::design_gates::RECORD, area: "tasks", keywords: "record design review architecture critic verdict rejected advice",
        example: r#"{"id":"API-12","role":"architecture","verdict":"approved","compatibility":"Both readers accept the old form.","rejected_advice":"None"}"# },
];

/// The cards a role finds (AC-174, AC-231).
pub fn cards(role: Role) -> impl Iterator<Item = &'static Card> {
    all_cards().filter(move |c| role.allows(c.name))
}

/// The areas a role is told of.
pub fn areas(role: Role) -> impl Iterator<Item = (&'static str, &'static str)> {
    let supervisor = role.is_supervisor();
    AREAS.iter().chain(&super::mcp_manager::AREAS).filter_map(move |&(area, what)| {
        cards(role).any(|c| c.area == area).then_some((area, if supervisor {
            match area {
                "tasks" => "this project's tasks",
                "runs" => "this project's runs",
                _ => what,
            }
        } else { what }))
    })
}

fn all_cards() -> impl Iterator<Item = &'static Card> {
    CARDS.iter().chain(&super::mcp_manager::CARDS).chain(super::mcp_supervisor::CARDS)
}

pub fn card(name: &str) -> Option<&'static Card> {
    all_cards().find(|c| c.name == name)
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
        all.extend(super::mcp_supervisor::tools());
        all
    })
}

pub fn definition(name: &str) -> Option<&'static Value> {
    definitions().iter().find(|d| d["name"] == name)
}

/// The Operator's start shares the tool name with the manager, but has a narrower contract.
pub fn definition_for(role: Role, name: &str) -> Option<&'static Value> {
    let d = definition(name).filter(|_| role.allows(name))?;
    if role == Role::Supervisor(Some(Level::Operator)) && name == "run_task" {
        static START: OnceLock<Value> = OnceLock::new();
        return Some(START.get_or_init(|| {
            let mut start = d.clone();
            start["description"] = serde_json::json!("Starts a free task in this project on the named pool agent. Dispatcher supplies a fixed prompt with the task ID.");
            let schema = &mut start["inputSchema"];
            if let Some(props) = schema["properties"].as_object_mut() {
                props.retain(|name, _| START_ARGS.contains(&name.as_str()));
            }
            schema["required"] = serde_json::json!(START_ARGS);
            schema["additionalProperties"] = serde_json::json!(false);
            start
        }));
    }
    if role == Role::Supervisor(Some(Level::Operator)) && name == "stop_task" {
        static STOP: OnceLock<Value> = OnceLock::new();
        return Some(STOP.get_or_init(super::mcp_supervisor::stop_tool));
    }
    Some(d)
}

pub fn example(role: Role, c: &Card) -> &'static str {
    match (role, c.name) {
        (Role::Supervisor(_), "run_task") => r#"{"id": "TERM-12", "agent": "Reviewer"}"#,
        (Role::Supervisor(_), "stop_task") => r#"{"id":"API-12","signal":"fingerprint","reason":"Cost exceeded the bound."}"#,
        _ => c.example,
    }
}

/// A role's search index: a tool's name, its keywords, its area and text, its arguments' names.
pub fn index(role: Role) -> &'static Index {
    static INDEXES: [OnceLock<Index>; 11] = [const { OnceLock::new() }; 11];
    let slot = match role {
        Role::Worker => 0,
        Role::Manager => 1,
        Role::Supervisor(Some(Level::Observer)) => 2,
        Role::Supervisor(Some(Level::Advisor)) => 3,
        Role::Supervisor(Some(Level::Operator)) => 4,
        Role::Supervisor(None) | Role::Reflection(false) | Role::Audit(false) | Role::DesignReview(false) => 5,
        Role::DesignReview(true) => 9,
        Role::Reflection(true) => 8,
        Role::Audit(true) => 10,
        Role::Token { manager: false } => 6,
        Role::Token { manager: true } => 7,
    };
    INDEXES[slot].get_or_init(|| {
        let docs: Vec<(String, String, &Card)> = cards(role).filter_map(|c| {
            let d = definition_for(role, c.name)?;
            let args: Vec<&str> = d["inputSchema"]["properties"].as_object().map(|p| p.keys().map(String::as_str).collect()).unwrap_or_default();
            Some((format!("{} {}", d["title"].as_str().unwrap_or(""), d["description"].as_str().unwrap_or("")), args.join(" "), c))
        }).collect();
        Index::new(&docs.iter().map(|(text, args, c)| Doc { name: c.name, fields: vec![(c.keywords, 2.0), (c.area, 1.0), (text, 1.0), (args, 1.0)] }).collect::<Vec<_>>())
    })
}
