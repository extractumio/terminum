// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use serde_json::json;

use super::*;

fn post(tool: &str, input: Value, response: Value) -> Value {
    json!({"hook_event_name": "PostToolUse", "session_id": "s1", "tool_name": tool, "tool_input": input, "tool_response": response})
}

#[test]
fn claude_tasks_add_and_set() {
    // payloads as Claude Code 2.1.310 sends them
    let c = post("TaskCreate", json!({"subject": "alpha", "description": "Task alpha"}), json!({"task": {"id": "1", "subject": "alpha"}}));
    assert_eq!(plan(&c), Some(json!({"op": "add", "id": "1", "text": "alpha"})));
    let u = post("TaskUpdate", json!({"taskId": "1", "status": "completed"}),
        json!({"success": true, "taskId": "1", "updatedFields": ["status"], "statusChange": {"from": "in_progress", "to": "completed"}}));
    assert_eq!(plan(&u), Some(json!({"op": "set", "id": "1", "status": "completed"})));
    let d = post("TaskUpdate", json!({"taskId": "2", "status": "deleted"}), json!({"success": true, "taskId": "2"}));
    assert_eq!(plan(&d), Some(json!({"op": "set", "id": "2", "status": "deleted"})));
    let r = post("TaskUpdate", json!({"taskId": "2", "subject": "beta two"}), json!({"success": true}));
    assert_eq!(plan(&r), Some(json!({"op": "set", "id": "2", "text": "beta two"})));
}

#[test]
fn todowrite_and_update_plan_are_whole_lists() {
    let t = post("TodoWrite", json!({"todos": [{"content": "alpha", "status": "completed", "activeForm": "Working on alpha"},
        {"content": "beta", "status": "pending", "activeForm": "Working on beta"}]}), json!({"oldTodos": [], "newTodos": []}));
    assert_eq!(plan(&t), Some(json!({"op": "all", "items": [{"text": "alpha", "status": "completed"}, {"text": "beta", "status": "pending"}]})));
    let p = post("update_plan", json!({"explanation": "x", "plan": [{"step": "read", "status": "in_progress"}]}), json!("Plan updated"));
    assert_eq!(plan(&p), Some(json!({"op": "all", "items": [{"text": "read", "status": "in_progress"}]})));
    let many: Vec<Value> = (0..80).map(|i| json!({"content": format!("t{i}"), "status": "pending"})).collect();
    assert_eq!(plan(&post("TodoWrite", json!({"todos": many}), json!({})))
        .unwrap()["items"].as_array().unwrap().len(), 50, "at most 50 items");
}

#[test]
fn a_failed_call_or_another_event_is_ignored() {
    let failed = post("TaskUpdate", json!({"taskId": "9", "status": "completed"}), json!({"success": false, "error": "Task not found"}));
    assert_eq!(plan(&failed), None);
    let mut pf = post("TaskCreate", json!({"subject": "a"}), json!({"task": {"id": "1"}}));
    pf["hook_event_name"] = json!("PostToolUseFailure");
    assert_eq!(plan(&pf), None);
    let mut pre = post("TodoWrite", json!({"todos": []}), Value::Null);
    pre["hook_event_name"] = json!("PreToolUse");
    assert_eq!(plan(&pre), None);
    assert_eq!(plan(&post("update_plan", json!({"plan": [{"step": "a", "status": "pending"}]}), json!("error: bad plan"))), None);
    assert_eq!(plan(&post("TaskCreate", json!({"subject": "a"}), json!({}))), None, "no id: never invented");
    // another tool's output never goes
    assert_eq!(plan(&post("Bash", json!({"command": "echo hi"}), json!("hi\n"))), None);
}

#[test]
fn text_is_cleaned_redacted_and_cut() {
    let key = format!("sk-{}", "ant-api03-abcdefghijkl");               // no secret's shape in the source
    let c = post("TaskCreate", json!({"subject": format!("rotate {key}\u{1b}[2J now")}), json!({"task": {"id": "3"}}));
    let t = plan(&c).unwrap()["text"].as_str().unwrap().to_string();
    assert!(!t.contains("ant-api03") && t.contains("[redacted]") && !t.contains('\u{1b}'), "{t}");
    let long = post("TodoWrite", json!({"todos": [{"content": "x".repeat(500), "status": "pending"}, {"content": " ", "status": "pending"}]}), json!({}));
    let items = plan(&long).unwrap()["items"].clone();
    assert_eq!((items[0]["text"].as_str().unwrap().chars().count(), items.as_array().unwrap().len()), (200, 1), "cut, an empty item dropped");
}

#[test]
fn bidi_controls_are_dropped_from_items() {
    let c = post("TaskCreate", json!({"subject": "a\u{202E}b\u{2067}c\u{200E}"}), json!({"task": {"id": "4"}}));
    assert_eq!(plan(&c).unwrap()["text"], "abc");
    let only = post("TodoWrite", json!({"todos": [{"content": "\u{202A}\u{202C}", "status": "pending"}]}), json!({}));
    assert_eq!(plan(&only).unwrap()["items"], json!([]), "nothing left: no item");
}
