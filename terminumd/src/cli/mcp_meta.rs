// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! Progressive disclosure (terminum-agents.md AC-155): tools/list carries search_tools,
//! get_tool_info, execute_tool and the few tools an agent needs without asking (DIRECT); the
//! rest of the catalog is found by search, described on demand and run through execute_tool,
//! so the catalog grows without a session paying for every schema at its start.

use serde_json::{json, Value};

use super::mcp_catalog as catalog;

pub const META: [&str; 3] = ["search_tools", "get_tool_info", "execute_tool"];
/// Listed in full: the plan, sent at every step with a schema the client checks. Every other tool
/// terminum's own words name is written there as its call, `execute_tool("finish_task", {id, result})`
/// (mcp_meta_tests.rs checks each against the tool's schema).
pub const DIRECT: [&str; 1] = ["update_session_plan"];
const LIMIT: usize = 5;
const LIMIT_MOST: usize = 10;
const QUERY_MOST: usize = 500;
const NAMES_MOST: usize = 5;

/// The areas, for search_tools' description; with their tools' names for an empty search.
fn areas(names: bool, manager: bool) -> String {
    catalog::areas(manager).map(|(area, what)| {
        let tools: Vec<&str> = catalog::cards(manager).filter(|c| c.area == *area).map(|c| c.name).collect();
        if names { format!("{area} ({what}): {}", tools.join(", ")) } else { format!("{area} ({what})") }
    }).collect::<Vec<_>>().join("; ")
}

/// The meta tools' definitions, their areas a role's own.
pub fn definitions(manager: bool) -> Vec<Value> {
    let read = super::mcp::hints(true, true);
    vec![
        json!({"name": "search_tools", "title": "Search terminum's tools",
            "description": format!("Finds terminum's tools for an action. query: the action as verb and object, in English, e.g. \"change a task's priority\", \"make a task a subtask of another\", \"attach a screenshot to a task\"; \
synonyms count. Answers the best tools with their arguments: run one with execute_tool. \
Areas: {}.", areas(false, manager)),
            "annotations": read,
            "inputSchema": {"type": "object", "required": ["query"], "properties": {
                "query": {"type": "string", "maxLength": QUERY_MOST, "description": "The action: verb and object, e.g. \"move a task under another task\"."},
                "limit": {"type": "integer", "minimum": 1, "maximum": LIMIT_MOST, "description": format!("At most this many tools (default {LIMIT}).")}}}}),
        json!({"name": "get_tool_info", "title": "Describe terminum's tools",
            "description": "terminum's tools' whole definitions by name, as search_tools found them: description, inputSchema, hints and an example call for execute_tool.",
            "annotations": read,
            "inputSchema": {"type": "object", "required": ["names"], "properties": {
                "names": {"type": "array", "minItems": 1, "maxItems": NAMES_MOST, "items": {"type": "string"}}}}}),
        json!({"name": "execute_tool", "title": "Run a terminum tool",
            "description": "Runs one of terminum's tools, found with search_tools, with its arguments as get_tool_info describes them; answers that tool's own result or error.",
            "annotations": super::mcp::hints(false, false),
            "inputSchema": {"type": "object", "required": ["name"], "properties": {
                "name": {"type": "string", "description": "The tool, e.g. update_task."},
                "arguments": {"type": "object", "description": "Its arguments, e.g. {\"id\": \"TERM-12\", \"severity\": \"P1\"}."}}}}),
    ]
}

/// tools/list: the meta tools, then the direct ones.
pub fn listed(manager: bool) -> Vec<Value> {
    let mut all = definitions(manager);
    all.extend(DIRECT.iter().filter_map(|n| catalog::definition(n).cloned()));
    all
}

/// An argument's shape in a signature: `name?` when optional, its values when they are few.
fn argument(name: &str, schema: &Value, required: bool) -> String {
    let opt = if required { "" } else { "?" };
    match schema.get("enum").and_then(Value::as_array) {
        Some(e) => format!("{name}{opt}: {}", e.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("|")),
        None if schema["type"] == "array" => format!("{name}{opt}: [...]"),
        None => format!("{name}{opt}"),
    }
}

/// A hit's line: its signature, its area, its description's first sentence: enough to call
/// execute_tool for most tools without get_tool_info.
fn summary(d: &Value) -> String {
    let name = d["name"].as_str().unwrap_or("");
    let description = d["description"].as_str().unwrap_or("");
    let first = description.split_inclusive(". ").next().unwrap_or(description).trim_end();
    let area = catalog::card(name).map_or("", |c| c.area);
    format!("{} [{area}]: {first}", signature(d))
}

/// `name(arg, arg?: a|b, …)`.
fn signature(d: &Value) -> String {
    let required: Vec<&str> = d["inputSchema"]["required"].as_array().map(|r| r.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let args: Vec<String> = d["inputSchema"]["properties"].as_object()
        .map(|p| p.iter().map(|(k, v)| argument(k, v, required.contains(&k.as_str()))).collect()).unwrap_or_default();
    format!("{}({})", d["name"].as_str().unwrap_or(""), args.join(", "))
}

/// Said under a tool's first refusal in a session that has not read its card (AC-155): its
/// signature, whole description and example call, the words get_tool_info gives, so the next call
/// can follow them without another round.
pub fn card_text(name: &str) -> Option<String> {
    let (d, c) = (catalog::definition(name)?, catalog::card(name)?);
    Some(format!("How to call it: {}: {}\nExample: execute_tool({})", signature(d), d["description"].as_str().unwrap_or(""), example_call(c)))
}

/// A card's example as execute_tool's arguments.
fn example_call(c: &catalog::Card) -> Value {
    json!({"name": c.name, "arguments": serde_json::from_str::<Value>(c.example).unwrap_or(Value::Null)})
}

/// get_tool_info's answer for one tool: its definition, area and an example call.
fn info(d: &Value) -> Value {
    let mut out = d.clone();
    if let Some(c) = d["name"].as_str().and_then(catalog::card) {
        out["area"] = json!(c.area);
        out["example"] = json!({"name": "execute_tool", "arguments": example_call(c)});
    }
    out
}

/// A name that is no tool of this role: why, and the nearest by search. A manager's tool is no tool to a worker.
fn unknown(name: &str, manager: bool) -> String {
    let near = catalog::index(manager).search(name, 3);
    let near = if near.is_empty() { String::new() } else { format!(" Nearest: {}.", near.join(", ")) };
    format!("No tool is named {name:?}.{near} search_tools finds a tool by what it does.")
}

/// search_tools: Ok(text) or Err(why).
pub fn search(args: &Value, manager: bool) -> Result<String, String> {
    let query = args.get("query").and_then(Value::as_str).map(str::trim).filter(|q| !q.is_empty())
        .ok_or("query is required: the action as a verb and its object, e.g. \"change a task's priority\"")?;
    if query.chars().count() > QUERY_MOST {
        return Err(format!("query is over {QUERY_MOST} characters: one action a query"));
    }
    let limit = match args.get("limit") {
        None | Some(Value::Null) => LIMIT,
        Some(v) => match v.as_u64() {
            Some(n @ 1..) if n as usize <= LIMIT_MOST => n as usize,
            _ => return Err(format!("limit {v} is not a whole number from 1 to {LIMIT_MOST}")),
        },
    };
    let hits = catalog::index(manager).search(query, limit);
    if hits.is_empty() {
        return Ok(format!("No tool matches {query:?}. Ask again with the action as a verb and its object in English, \
e.g. \"create a task\". The tools by area: {}.", areas(true, manager)));
    }
    let lines: Vec<String> = hits.iter().filter_map(|n| catalog::definition(n)).enumerate().map(|(i, d)| format!("{}. {}", i + 1, summary(d))).collect();
    Ok(format!("Tools for {query:?}, best first (? optional):\n{}\nRun one: execute_tool {{name, arguments}}; get_tool_info {{names}} gives its whole description, schema and an example.", lines.join("\n")))
}

/// get_tool_info: its text and the tools it described, or why not.
pub fn describe(args: &Value, manager: bool) -> Result<(String, Vec<&'static str>), String> {
    let names: Vec<&str> = match args.get("names") {
        Some(Value::String(n)) => vec![n.as_str()],
        Some(Value::Array(a)) if !a.is_empty() && a.len() <= NAMES_MOST && a.iter().all(Value::is_string) => a.iter().filter_map(Value::as_str).collect(),
        _ => return Err(format!("names is required: 1–{NAMES_MOST} tool names from search_tools")),
    };
    let (mut out, mut described) = (Vec::new(), Vec::new());
    for n in names {
        match catalog::definition(n).filter(|_| manager || !catalog::manager_only(n)) {
            Some(d) => {
                out.push(info(d));
                described.extend(catalog::card(n).map(|c| c.name));
            }
            None if META.contains(&n) => return Err(format!("{n} is listed by tools/list already.")),
            None => return Err(unknown(n, manager)),
        }
    }
    Ok((Value::Array(out).to_string(), described))
}

/// execute_tool: the catalog tool to run and its arguments, or why not.
pub fn target(args: &Value, manager: bool) -> Result<(&'static str, Value), String> {
    let name = args.get("name").and_then(Value::as_str).filter(|n| !n.is_empty()).ok_or("name is required: a tool from search_tools")?;
    if META.contains(&name) {
        return Err(format!("{name} is called directly, not through execute_tool."));
    }
    let card = catalog::card(name).filter(|c| manager || !catalog::manager_only(c.name)).ok_or_else(|| unknown(name, manager))?;
    let arguments = match args.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(a @ Value::Object(_)) => a.clone(),
        Some(_) => return Err("arguments must be an object: the tool's arguments, as get_tool_info describes them".into()),
    };
    Ok((card.name, arguments))
}

#[cfg(test)]
#[path = "mcp_meta_tests.rs"]
mod tests;
