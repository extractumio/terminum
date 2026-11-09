// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use serde_json::json;

use super::*;
use crate::cli::mcp;

/// The cases whose tool is not where `found` wants it in the first three hits.
fn misses(cases: &[(&str, &str)], found: impl Fn(&[String], &str) -> bool) -> String {
    cases.iter().filter_map(|(query, want)| {
        let got = catalog::index().search(query, 3);
        (!found(&got, want)).then(|| format!("{query:?}: {got:?}, want {want}"))
    }).collect::<Vec<_>>().join("\n")
}

/// What an agent asks, and the tool it must get first: the search's specification. A new tool
/// adds its phrasings here.
const GOLDEN: [(&str, &str); 44] = [
    ("change a task's priority", "update_task"),
    ("bump the severity of a ticket", "update_task"),
    ("reprioritize an issue", "update_task"),
    ("lower priority to P3", "update_task"),
    ("make a task a subtask of another", "update_task"),
    ("make it a subtask of T-3", "update_task"),
    ("move a task under another task", "update_task"),
    ("take a task out of its parent", "update_task"),
    ("rename a task", "update_task"),
    ("edit a task's text", "update_task"),
    ("archive a done task", "update_task"),
    ("delete the task", "update_task"),
    ("remove a card from the board", "update_task"),
    ("create a task", "create_task"),
    ("file a bug", "create_task"),
    ("open a new ticket", "create_task"),
    ("add a feature request", "create_task"),
    ("record tech debt left to do", "create_task"),
    ("list the open tasks", "list_tasks"),
    ("see the board", "list_tasks"),
    ("find tasks with severity P1", "list_tasks"),
    ("what is in the backlog", "list_tasks"),
    ("read a task", "get_task"),
    ("show a ticket's details and comments", "get_task"),
    ("claim a task", "take_task"),
    ("assign the task to me", "take_task"),
    ("start working on a task", "take_task"),
    ("I need a human", "ask_human"),
    ("block the task waiting for a person", "move_task"),
    ("move my task to another column", "move_task"),
    ("change a task's status to failed", "move_task"),
    ("mark my task done", "finish_task"),
    ("close the ticket with its result", "finish_task"),
    ("complete my task", "finish_task"),
    ("leave a comment on a task", "add_comment"),
    ("reply on a ticket", "add_comment"),
    ("add a note to an issue", "add_comment"),
    ("attach a screenshot to a task", "attach_file"),
    ("upload a log file", "attach_file"),
    ("download the attachment", "get_file"),
    ("get a task's file", "get_file"),
    ("update my plan", "update_session_plan"),
    ("report progress and the ETA", "update_session_plan"),
    ("what is my plan", "get_session_plan"),
];

/// Looser phrasings: the tool is among the first three, the agent choosing by their lines.
const NEAR: [(&str, &str); 10] = [
    ("prioritize this bug higher", "update_task"),
    ("split a task into subtasks", "update_task"),
    ("group tasks under an epic", "update_task"),
    ("escalate to the user", "move_task"),
    ("give up on the task, it failed", "move_task"),
    ("postpone a task", "move_task"),
    ("what should I work on next", "list_tasks"),
    ("see the comments of a task", "get_task"),
    ("mark a step of my plan done", "update_session_plan"),
    ("add a document to the ticket", "attach_file"),
];

#[test]
fn ac155_a_looser_phrasing_finds_its_tool_among_three() {
    let wrong = misses(&NEAR, |got, want| got.iter().any(|g| g == want));
    assert!(wrong.is_empty(), "{wrong}");
}

#[test]
fn ac155_each_phrasing_finds_its_tool_first() {
    let wrong = misses(&GOLDEN, |got, want| got.first().is_some_and(|g| g == want));
    assert!(wrong.is_empty(), "{wrong}");
}

#[test]
fn ac155_every_tool_has_its_card_and_a_valid_example() {
    let cards: Vec<&str> = catalog::CARDS.iter().map(|c| c.name).collect();
    assert_eq!(cards, mcp::tool_names(), "one card for each tool, in its order");
    for c in catalog::CARDS {
        let example: Value = serde_json::from_str(c.example).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        assert!(mcp::checked(c.name, &example).is_ok(), "{}'s example is refused", c.name);
        assert!(catalog::AREAS.iter().any(|(a, _)| *a == c.area), "{}: no area {}", c.name, c.area);
    }
}

/// Each place terminum's words name `name` outside a call: none may be, but for a listed tool. A call
/// is `execute_tool("name", {a, b?})`: its arguments the tool's own, every required one there, `?` on
/// an optional one.
fn uncalled(words: &str, name: &str) -> Vec<String> {
    let text = words.replace("\\\"", "\"").replace("{{", "{").replace("}}", "}");
    let schema = &catalog::definition(name).unwrap()["inputSchema"];
    let required: Vec<&str> = schema["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let mut wrong = Vec::new();
    for (at, _) in text.match_indices(name) {
        let after = &text[at + name.len()..];
        let word = !text[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') && !after.starts_with(|c: char| c.is_alphanumeric() || c == '_');
        if !word {
            continue;
        }
        let Some(args) = text[..at].ends_with("execute_tool(\"").then(|| after.strip_prefix("\", {")).flatten().and_then(|a| a.split_once('}')) else {
            wrong.push(format!("{name} at {at}: not as execute_tool(\"{name}\", {{...}})"));
            continue;
        };
        let given: Vec<(&str, bool)> = args.0.split(',').map(str::trim).map(|a| (a.trim_end_matches('?'), a.ends_with('?'))).collect();
        for (arg, optional) in &given {
            if schema["properties"].get(arg).is_none() || (*optional && required.contains(arg)) {
                wrong.push(format!("{name} at {at}: {arg} is not its argument as written"));
            }
        }
        wrong.extend(required.iter().filter(|r| !given.iter().any(|(g, _)| g == *r)).map(|r| format!("{name} at {at}: {r} is required")));
    }
    wrong
}

#[test]
fn ac155_terminum_s_words_call_a_tool_not_listed_by_execute_tool() {
    let words = [mcp::INSTRUCTIONS, include_str!("../dispatcher/press.rs"), include_str!("../dispatcher/acts.rs"), crate::plans::REMINDER];
    let wrong: Vec<String> = mcp::tool_names().into_iter().filter(|n| !DIRECT.contains(n)).flat_map(|n| words.iter().flat_map(move |w| uncalled(w, n))).collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    let listed: Vec<String> = listed().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert_eq!(listed, ["search_tools", "get_tool_info", "execute_tool", "update_session_plan"]);
}

#[test]
fn ac155_a_name_outside_its_call_or_a_wrong_argument_is_found() {
    assert_eq!(uncalled("call get_task now", "get_task").len(), 1);
    assert_eq!(uncalled(r#"execute_tool("get_task", {page})"#, "get_task").len(), 1, "id is required");
    assert_eq!(uncalled(r#"execute_tool("get_task", {id, page, task})"#, "get_task").len(), 1, "task is no argument of it");
    assert_eq!(uncalled(r#"execute_tool("get_task", {id?})"#, "get_task").len(), 1, "a required one is not optional");
    assert!(uncalled(r#"execute_tool("get_task", {id, page?})"#, "get_task").is_empty());
}

#[test]
fn ac155_search_answers_signatures_or_the_areas() {
    let out = search(&json!({"query": "move my task to another column"})).unwrap();
    assert!(out.starts_with("Tools for \"move my task to another column\", best first"), "{out}");
    assert!(out.contains("1. move_task(id, to: TODO|IN PROGRESS|WAIT FOR HUMAN|FAILED|DONE) [tasks]: Moves this session's task to another column (status)."), "{out}");
    assert!(out.contains("execute_tool"), "{out}");
    let none = search(&json!({"query": "zzz qqq"})).unwrap();
    assert!(none.starts_with("No tool matches \"zzz qqq\"") && none.contains("files (on a task: attach, get): attach_file, get_file"), "{none}");
    assert_eq!(search(&json!({"query": "find a task", "limit": 1})).unwrap().lines().filter(|l| l.starts_with(char::is_numeric)).count(), 1);
    assert!(search(&json!({})).unwrap_err().starts_with("query is required"));
    assert!(search(&json!({"query": "  "})).unwrap_err().starts_with("query is required"));
    assert!(search(&json!({"query": "x".repeat(501)})).unwrap_err().contains("over 500"));
    for bad in [json!(0), json!(11), json!("3"), json!(-1)] {
        assert!(search(&json!({"query": "a task", "limit": bad})).unwrap_err().starts_with("limit"), "{bad}");
    }
}

#[test]
fn ac155_get_tool_info_gives_schema_hints_and_an_example() {
    let out: Value = serde_json::from_str(&describe(&json!({"names": ["update_task", "list_tasks"]})).unwrap()).unwrap();
    assert_eq!(out[0]["name"], "update_task");
    assert_eq!(out[0]["inputSchema"]["properties"]["severity"]["enum"], json!(["P0", "P1", "P2", "P3"]));
    assert_eq!(out[0]["example"]["name"], "execute_tool");
    assert_eq!(out[0]["example"]["arguments"]["name"], "update_task");
    assert_eq!(out[1]["annotations"]["readOnlyHint"], true);
    assert_eq!(serde_json::from_str::<Value>(&describe(&json!({"names": "get_file"})).unwrap()).unwrap()[0]["name"], "get_file", "a bare name is a list of one");
    assert!(describe(&json!({"names": ["update_tasks"]})).unwrap_err().contains("Nearest: update_task"));
    assert!(describe(&json!({"names": []})).is_err());
    assert!(describe(&json!({"names": ["a", "b", "c", "d", "e", "f"]})).is_err());
}

#[test]
fn ac155_execute_tool_runs_a_catalog_tool_only() {
    assert_eq!(target(&json!({"name": "list_tasks", "arguments": {"status": "TODO"}})).unwrap(), ("list_tasks", json!({"status": "TODO"})));
    assert_eq!(target(&json!({"name": "get_session_plan"})).unwrap(), ("get_session_plan", json!({})), "arguments default to none");
    assert!(target(&json!({"name": "execute_tool", "arguments": {}})).unwrap_err().contains("directly"));
    assert!(target(&json!({"name": "search_tools"})).unwrap_err().contains("directly"));
    assert!(target(&json!({"name": "drop_table"})).unwrap_err().starts_with("No tool is named \"drop_table\""));
    assert!(target(&json!({"name": "add_comment", "arguments": "x"})).unwrap_err().starts_with("arguments must be an object"));
    assert!(target(&json!({})).unwrap_err().starts_with("name is required"));
}
