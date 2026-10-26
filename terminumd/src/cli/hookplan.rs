// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The `plan` field of a hook event: what an agent's own plan tool changed, read from a
//! successful PostToolUse (Claude Code TaskCreate, TaskUpdate, TodoWrite; Codex update_plan).
//! Only the items' words and statuses go, control characters dropped, secrets redacted, cut
//! short; never anything else of the tool's output.

use serde_json::{json, Value};

use crate::plans::rules::{ITEMS_MOST as ITEMS, ITEM_TEXT as TEXT};

fn words(v: Option<&Value>) -> Option<String> {
    let s = crate::plans::rules::clean_with(v?.as_str()?, false);
    (!s.is_empty()).then(|| s.chars().take(TEXT).collect())
}

fn status(v: Option<&Value>) -> Option<String> {
    v?.as_str().filter(|s| !s.is_empty() && s.len() <= 20).map(str::to_string)
}

/// A whole list of `{<text>, status}` objects as an "all" op.
fn all(list: &Value, text: &str) -> Option<Value> {
    let items: Vec<Value> = list.as_array()?.iter().take(ITEMS).filter_map(|i| {
        Some(json!({"text": words(i.get(text))?, "status": status(i.get("status")).unwrap_or_else(|| "pending".into())}))
    }).collect();
    Some(json!({"op": "all", "items": items}))
}

fn id(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) if !s.is_empty() => Some(s.chars().take(40).collect()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// The plan op of `input`, an agent's event; None unless it is a successful PostToolUse of a
/// plan tool.
pub fn plan(input: &Value) -> Option<Value> {
    if input.get("hook_event_name").and_then(Value::as_str) != Some("PostToolUse") {
        return None;
    }
    let tin = input.get("tool_input")?;
    let out = input.get("tool_response").unwrap_or(&Value::Null);
    match input.get("tool_name").and_then(Value::as_str)? {
        "TaskCreate" => Some(json!({"op": "add", "id": id(out.pointer("/task/id"))?, "text": words(tin.get("subject"))?})),
        "TaskUpdate" => {
            if out.get("success").and_then(Value::as_bool) != Some(true) {
                return None;
            }
            let mut op = json!({"op": "set", "id": id(tin.get("taskId"))?});
            if let Some(s) = status(tin.get("status")) {
                op["status"] = json!(s);
            }
            if let Some(t) = words(tin.get("subject")) {
                op["text"] = json!(t);
            }
            Some(op)
        }
        "TodoWrite" => all(tin.get("todos")?, "content"),
        "update_plan" => {
            let ok = match out {
                Value::String(s) => s.trim() == "Plan updated",
                Value::Object(o) => o.get("success").and_then(Value::as_bool) != Some(false) && o.get("error").is_none(),
                _ => false,
            };
            if !ok {
                return None;
            }
            all(tin.get("plan")?, "step")
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "hookplan_tests.rs"]
mod tests;
