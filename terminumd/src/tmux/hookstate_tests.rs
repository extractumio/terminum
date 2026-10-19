// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use super::*;
use serde_json::json;

const K: &str = "tmux:local:terminum:%3";

fn ev(agent: &str, event: &str, extra: Value) -> Value {
    let mut v = json!({"agent": agent, "event": event, "session": "s1"});
    v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
    v
}

#[test]
fn a_turn_is_working_then_done_with_its_tool_and_subagents() {
    let h = Hooks::default();
    let t = Instant::now();
    assert!(h.apply(K, &ev("claude", "UserPromptSubmit", json!({"turn": "p1", "prompt": "fix"})), t));
    h.apply(K, &ev("claude", "PreToolUse", json!({"turn": "p1", "tool": "Bash: npm test"})), t);
    h.apply(K, &ev("claude", "SubagentStart", json!({"turn": "p1", "sub": "a1"})), t);
    let v = h.view(K, "2.1.303", t).unwrap();
    assert_eq!((v.state, v.tool.as_str(), v.subagents), (State::Working, "Bash: npm test", 1));
    h.apply(K, &ev("claude", "Stop", json!({"turn": "p1"})), t);
    let v = h.view(K, "2.1.303", t).unwrap();
    assert_eq!((v.state, v.subagents, v.tool.as_str()), (State::Done, 0, ""));
}

#[test]
fn waiting_for_an_approval_or_an_answer_and_a_failure() {
    let h = Hooks::default();
    let t = Instant::now();
    assert!(!h.apply(K, &ev("claude", "PermissionRequest", json!({"tool": "Bash: rm -rf"})), t), "no start or prompt seen: no record");
    h.apply(K, &ev("claude", "UserPromptSubmit", json!({"turn": "p1"})), t);
    h.apply(K, &ev("claude", "PermissionRequest", json!({"tool": "Bash: rm -rf"})), t);
    assert_eq!(h.view(K, "c", t).map(|v| (v.state, v.why)), Some((State::Waiting, "Approve Bash: rm -rf".to_string())));
    h.apply(K, &ev("claude", "PreToolUse", json!({"tool": "AskUserQuestion", "message": "Which one?"})), t);
    assert_eq!(h.view(K, "c", t).map(|v| v.why), Some("Which one?".to_string()));
    h.apply(K, &ev("claude", "StopFailure", json!({"error": "rate_limit"})), t);
    assert_eq!(h.view(K, "c", t).map(|v| (v.state, v.why)), Some((State::Failed, "rate_limit".to_string())));
}

#[test]
fn a_late_working_event_of_a_stopped_turn_changes_nothing() {
    let h = Hooks::default();
    let t = Instant::now();
    h.apply(K, &ev("claude", "UserPromptSubmit", json!({"turn": "p1"})), t);
    h.apply(K, &ev("claude", "Stop", json!({"turn": "p1"})), t);
    assert!(!h.apply(K, &ev("claude", "PostToolUse", json!({"turn": "p1"})), t), "background hooks come out of order");
    assert!(!h.apply(K, &ev("claude", "PermissionRequest", json!({"turn": "p1", "tool": "Bash"})), t), "nor a late approval");
    assert!(!h.apply(K, &ev("claude", "Notification", json!({"turn": "p1", "kind": "permission_prompt"})), t));
    assert_eq!(h.view(K, "c", t).unwrap().state, State::Done);
    h.apply(K, &ev("claude", "SessionEnd", json!({})), t);
    assert!(!h.apply(K, &ev("claude", "PostToolUse", json!({"turn": "p1"})), t), "a late event of an ended session makes no record");
    assert!(!h.has(K));
}

#[test]
fn leases_run_out_and_a_new_program_or_session_starts_anew() {
    let h = Hooks::default();
    let t = Instant::now();
    h.apply(K, &ev("claude", "UserPromptSubmit", json!({"turn": "p1"})), t);
    assert!(h.view(K, "2.1.303", t + LEASE - Duration::from_secs(1)).is_some());
    assert!(h.view(K, "2.1.303", t + LEASE).is_none(), "an interrupt fires no Stop: the screen says again");
    h.apply(K, &ev("claude", "PermissionRequest", json!({"tool": "Bash"})), t);
    assert!(h.view(K, "2.1.303", t + WAIT_LEASE).is_none());
    h.apply(K, &ev("claude", "Stop", json!({})), t);
    assert!(h.view(K, "zsh", t).is_none(), "the program changed: its hooks no longer hold");
    assert!(!h.has(K));
    h.apply(K, &ev("claude", "Stop", json!({})), t);
    h.apply(K, &json!({"agent": "claude", "event": "UserPromptSubmit", "session": "s2"}), t);
    assert_eq!(h.view(K, "c", t).unwrap().state, State::Working, "another session: a record of its own");
    h.apply(K, &ev("claude", "SessionEnd", json!({})), t);
    assert!(h.view(K, "c", t).is_none());
}

#[test]
fn codex_done_is_the_screens_and_unknown_events_change_nothing() {
    let h = Hooks::default();
    let t = Instant::now();
    h.apply(K, &ev("codex", "UserPromptSubmit", json!({"turn": "t1"})), t);
    assert_eq!(h.view(K, "node", t).unwrap().agent, "codex");
    h.apply(K, &ev("codex", "Stop", json!({"turn": "t1"})), t);
    assert!(h.view(K, "node", t).is_none(), "Codex has no failure event: done may be a failure on the screen");
    assert!(!h.apply(K, &ev("codex", "SomethingNew", json!({})), t));
    assert!(!h.apply(K, &json!({"agent": "other", "event": "Stop"}), t));
    h.retain("tmux:local:terminum:", &[]);
    assert!(!h.has(K), "a pane that is gone is forgotten");
}
