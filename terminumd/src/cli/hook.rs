// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! `terminum hook claude|codex` (AC-39): what a coding agent's hooks run (`terminum hooks
//! install`). It only observes: it reads the event the agent gives on stdin (to its end, so the
//! agent's write never breaks), keeps a few fields of it, cut short (never a tool's output, a
//! file's text or the agent's answer), and posts them to terminumd's private socket with a
//! budget of 150 ms; it prints nothing and always exits 0, so the agent goes on as without it.
//! The pane is the one tmux names in the agent's environment (TMUX, TMUX_PANE); outside tmux
//! nothing is sent.

use std::io::Read;
use std::time::Duration;

use serde_json::{json, Map, Value};

/// The most of the event read (its rest is read and dropped).
const KEEP: usize = 1 << 20;
const BUDGET: Duration = Duration::from_millis(150);

/// Runs the hook for `agent`; always 0 when an agent runs it. Run by a person (an unknown
/// agent, or a terminal for stdin), it says what to run instead: it would wait for the keys.
pub fn main(agent: &str) -> i32 {
    // SAFETY: isatty has no preconditions
    let by_hand = unsafe { libc::isatty(0) } == 1;
    if let Some(why) = refused(agent, by_hand) {
        eprintln!("{why}");
        return 2;
    }
    let mut kept = Vec::new();
    let mut buf = [0u8; 65536];
    let mut stdin = std::io::stdin().lock();
    while let Ok(n) = stdin.read(&mut buf) {
        if n == 0 {
            break;
        }
        if kept.len() < KEEP {
            kept.extend_from_slice(&buf[..n.min(KEEP - kept.len())]);
        }
    }
    let (Ok(input), Ok(tmux), Ok(pane)) = (serde_json::from_slice::<Value>(&kept), std::env::var("TMUX"), std::env::var("TMUX_PANE")) else { return 0 };
    if let Some(ev) = event(agent, &input, &tmux, &pane) {
        let sock = crate::sock::path(&crate::config::app_dir());
        let _ = super::sockhttp::call(&sock, "POST", "/hook", Some(&ev), BUDGET);
    }
    0
}

/// Why `terminum hook <agent>` is not an agent's run (`by_hand`: its stdin is a terminal).
pub fn refused(agent: &str, by_hand: bool) -> Option<String> {
    let how = "terminum hook is what a coding agent's hooks run; to set them up: terminum hooks install";
    if crate::tmux::hookstate::agent(agent).is_none() {
        return Some(format!("usage: terminum hook claude|codex (an event on stdin)\n{how}"));
    }
    by_hand.then(|| how.to_string())
}

/// At most `n` characters of `s`, control characters dropped.
fn cut(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\n').take(n).collect()
}

fn text(v: &Value, k: &str, n: usize) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(|s| cut(s, n)).filter(|s| !s.is_empty())
}

/// What a tool does, in a few words: a shell's command (its first two words), a file's name,
/// else the tool's name. Never its whole input.
pub fn tool_summary(name: &str, input: &Value) -> String {
    let command = match input.get("command") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")),
        _ => None,
    };
    let what = if let Some(c) = command {
        // a word with a value (TOKEN=abc, --password=x) keeps only its name
        c.split_whitespace().take(2).map(|w| w.split_once('=').map_or(w.to_string(), |(k, _)| format!("{k}=…"))).collect::<Vec<_>>().join(" ")
    } else if let Some(p) = input.get("file_path").or_else(|| input.get("path")).and_then(Value::as_str) {
        p.rsplit('/').next().unwrap_or(p).to_string()
    } else {
        String::new()
    };
    cut(&crate::sent::redact(&if what.is_empty() { name.to_string() } else { format!("{name}: {what}") }), 80)
}

/// The fields terminumd gets of `input`, the agent's event, from pane `pane` of the tmux
/// whose socket `tmux` names ("/private/tmp/tmux-501/terminum,123,0"); None when it is no event.
pub fn event(agent: &str, input: &Value, tmux: &str, pane: &str) -> Option<Value> {
    let socket = tmux.split(',').next()?.rsplit('/').next()?.to_string();
    let name = text(input, "hook_event_name", 40)?;
    if crate::tmux::hookstate::agent(agent).is_none() || socket.is_empty() || !pane.starts_with('%') || !pane[1..].bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut out = Map::new();
    out.insert("agent".into(), json!(agent));
    out.insert("socket".into(), json!(cut(&socket, 64)));
    out.insert("pane".into(), json!(pane));
    out.insert("event".into(), json!(name));
    for (from, to, n) in [("session_id", "session", 100), ("agent_id", "sub", 100), ("agent_type", "subtype", 60),
                          ("notification_type", "kind", 40), ("error", "error", 60), ("cwd", "cwd", 1000), ("model", "model", 80)] {
        if let Some(v) = text(input, from, n) {
            out.insert(to.into(), json!(v));
        }
    }
    if let Some(t) = text(input, "turn_id", 100).or_else(|| text(input, "prompt_id", 100)) {
        out.insert("turn".into(), json!(t));
    }
    let tool = input.get("tool_name").and_then(Value::as_str);
    if let Some(t) = tool {
        out.insert("tool".into(), json!(tool_summary(t, input.get("tool_input").unwrap_or(&Value::Null))));
    }
    // a question it asks the user (Claude Code's AskUserQuestion), else a notification's words
    let asked = input.pointer("/tool_input/questions/0/question").and_then(Value::as_str).filter(|_| tool == Some("AskUserQuestion"));
    if let Some(m) = asked.map(|q| cut(q, 200)).or_else(|| text(input, "message", 200)) {
        out.insert("message".into(), json!(m));
    }
    if name == "UserPromptSubmit"
        && let Some(p) = input.get("prompt").and_then(Value::as_str) {
            out.insert("prompt".into(), json!(p.chars().take(crate::sent::TEXT).collect::<String>()));
        }
    Some(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TMUX: &str = "/private/tmp/tmux-501/terminum,123,0";

    #[test]
    fn only_a_few_fields_go_and_never_a_tools_output() {
        let input = json!({"hook_event_name": "PostToolUse", "session_id": "s1", "prompt_id": "p1", "cwd": "/Users/alex/api",
            "tool_name": "Bash", "tool_input": {"command": "npm test --watch=false SECRET=abc"}, "tool_response": {"stdout": "all the output"},
            "transcript_path": "/Users/alex/.claude/x.jsonl", "last_assistant_message": "done"});
        let ev = event("claude", &input, TMUX, "%3").unwrap();
        assert_eq!(ev, json!({"agent": "claude", "socket": "terminum", "pane": "%3", "event": "PostToolUse", "session": "s1", "turn": "p1",
            "cwd": "/Users/alex/api", "tool": "Bash: npm test"}));
        assert!(!ev.to_string().contains("output") && !ev.to_string().contains("SECRET") && !ev.to_string().contains("transcript"));
    }

    #[test]
    fn a_question_a_prompt_a_failure_a_file() {
        let ask = json!({"hook_event_name": "PreToolUse", "tool_name": "AskUserQuestion", "tool_input": {"questions": [{"question": "Which one?\u{1b}[2J"}]}});
        assert_eq!(event("claude", &ask, TMUX, "%1").unwrap()["message"], "Which one?[2J", "control characters dropped");
        let p = json!({"hook_event_name": "UserPromptSubmit", "prompt": "fix the tests", "turn_id": "t9"});
        let ev = event("codex", &p, TMUX, "%1").unwrap();
        assert_eq!((ev["prompt"].as_str(), ev["turn"].as_str()), (Some("fix the tests"), Some("t9")));
        let f = json!({"hook_event_name": "StopFailure", "error": "rate_limit"});
        assert_eq!(event("claude", &f, TMUX, "%1").unwrap()["error"], "rate_limit");
        assert_eq!(tool_summary("Edit", &json!({"file_path": "/Users/alex/api/src/main.rs", "new_string": "secret"})), "Edit: main.rs");
        assert_eq!(tool_summary("exec_command", &json!({"command": ["bash", "-lc", "ls"]})), "exec_command: bash -lc");
        assert_eq!(tool_summary("Bash", &json!({"command": "export TOKEN=sk-abc123"})), "Bash: export TOKEN=[redacted]", "a value is never sent");
        let key = format!("sk-{}", "ant-api03-abcdefghijkl");               // no secret's shape in the source
        assert_eq!(tool_summary("Bash", &json!({"command": format!("echo {key}")})), "Bash: echo [redacted]");
        assert_eq!(tool_summary("Bash", &json!({"command": "make X=1"})), "Bash: make X=…");
    }

    #[test]
    fn run_by_a_person_it_says_what_to_run() {
        assert!(refused("install", false).unwrap().contains("terminum hooks install"), "terminum hook install: a typo of hooks");
        assert!(refused("claude", true).unwrap().contains("terminum hooks install"), "a terminal for stdin: it would wait for the keys");
        assert_eq!(refused("codex", false), None);
    }

    #[test]
    fn nothing_without_a_pane_an_event_or_a_known_agent() {
        let e = json!({"hook_event_name": "Stop"});
        assert!(event("claude", &e, TMUX, "").is_none());
        assert!(event("claude", &e, TMUX, "%x").is_none());
        assert!(event("other", &e, TMUX, "%1").is_none());
        assert!(event("claude", &json!({"x": 1}), TMUX, "%1").is_none());
        assert!(event("claude", &e, "", "%1").is_none());
    }
}
