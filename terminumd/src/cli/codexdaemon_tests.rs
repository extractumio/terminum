// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-207: a shared Codex app-server is told apart, and under one no pane is taken from the environment.
use serde_json::{json, Value};

use super::super::mcp::{answer, Env, NOT_IN_PANE};
use super::super::sockhttp::Down;
use super::*;

fn w(s: &str) -> Vec<&str> {
    s.split(' ').collect()
}

#[test]
fn ac207_a_listening_app_server_is_shared() {
    for line in ["/Users/alex/.codex/packages/app-server-daemon/releases/0.171.0-aarch64-apple-darwin/bin/codex app-server --listen unix:// --analytics-default-enabled",
                 "codex app-server --listen unix:///Users/alex/.codex/app.sock", "codex app-server --listen=ws://127.0.0.1:4500",
                 "node /opt/homebrew/bin/codex app-server --listen unix://"] {
        assert!(shared(&w(line)), "{line}");
    }
}

#[test]
fn ac207_a_stdio_app_server_or_another_codex_is_not_shared() {
    for line in ["codex app-server", "codex app-server --listen stdio://", "codex app-server --stdio", "codex app-server --listen off",
                 "codex app-server --listen", "codex --no-alt-screen", "codex app-server daemon pid-update-loop",
                 "codex exec --listen unix:// fix", "claude app-server --listen unix://", "codex-helper app-server --listen unix://", ""] {
        assert!(!shared(&w(line)), "{line}");
    }
}

/// Run as the child of `ac207_a_shared_app_servers_child_takes_no_pane`: what `terminum mcp`,
/// `terminum tool` and the hooks take from this environment.
#[test]
fn child_says_its_pane() {
    if std::env::var_os("TM_TEST_SHARED_CHILD").is_none() {
        return;
    }
    let get = |k: &str| std::env::var(k).ok();
    let env = super::super::mcp::env_of(get);
    let hook = |agent| super::super::hook::takes_pane(agent);
    println!("said: shared={} pane={} codex={} claude={}", env.shared, super::super::mcp::pane_of(&env).is_some(), hook("codex"), hook("claude"));
}

/// The child's line, its parent a shell whose words are `arg0 -c … app-server --listen <listen>`, in pane %3.
fn child_under(arg0: &str, listen: &str) -> String {
    use std::os::unix::process::CommandExt;
    let out = Command::new("/bin/sh").arg0(arg0)
        .args(["-c", "\"$TM_TEST_EXE\" --exact cli::codexdaemon::tests::child_says_its_pane --nocapture -q; exit $?", "app-server", "--listen", listen])
        .env("TM_TEST_EXE", std::env::current_exe().unwrap()).env("TM_TEST_SHARED_CHILD", "1")
        .env("TMUX", "/private/tmp/tmux-501/terminum,123,0").env("TMUX_PANE", "%3").output().unwrap();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(out.status.success(), "{text}{}", String::from_utf8_lossy(&out.stderr));
    text.lines().find_map(|l| l.strip_prefix("said: ")).unwrap_or_else(|| panic!("no line: {text}")).to_string()
}

#[test]
fn ac207_a_shared_app_servers_child_takes_no_pane() {
    assert_eq!(child_under("/Users/alex/.codex/packages/app-server-daemon/bin/codex", "unix://"), "shared=true pane=false codex=false claude=true",
               "a pane in its environment is not the caller's");
    assert_eq!(child_under("codex", "stdio://"), "shared=false pane=true codex=true claude=true", "an editor's own app-server");
    assert_eq!(child_under("sh", "unix://"), "shared=false pane=true codex=true claude=true", "another program");
}

#[test]
fn ac207_the_tests_ancestors_are_no_shared_app_server() {
    assert!(!under_shared(), "cargo's test runner");
    let me = std::process::id();
    assert_eq!(parent_of(me), Some(std::os::unix::process::parent_id()));
    assert_eq!(parent_of(u32::MAX), None, "no such process");
}

fn no_call(_: &Value) -> Result<(u16, Option<Value>), Down> {
    panic!("nothing may be forwarded outside a pane")
}

/// The result of `method` with `params` for `terminum mcp` outside a pane, under a shared app-server or not.
fn asked(shared: bool, method: &str, params: Value) -> Value {
    let env = Env { shared, ..Default::default() };
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).to_string();
    serde_json::from_str::<Value>(&answer(&line, &env, &no_call).expect("an answer")).unwrap()["result"].clone()
}

#[test]
fn ac207_under_a_shared_app_server_the_session_is_told_why_it_has_no_tools() {
    assert!(asked(true, "initialize", json!({})).get("instructions").is_none(), "the app's own conversations get no words");
    let tools = asked(true, "tools/list", json!({}))["tools"].clone();
    assert_eq!(tools.as_array().map(Vec::len), Some(1), "{tools}");
    assert_eq!(tools[0]["name"], TOOL);
    assert_eq!(tools[0]["description"], SAID);
    let r = asked(true, "tools/call", json!({"name": TOOL, "arguments": {}}));
    assert_eq!(r, json!({"content": [{"type": "text", "text": SAID}], "isError": false}));
}

#[test]
fn ac207_under_a_shared_app_server_terminums_tools_answer_why_and_forward_nothing() {
    for (name, args) in [("update_session_plan", json!({"items": [{"text": "a", "status": "pending"}]})),
                         ("search_tools", json!({"query": "a task"})), ("execute_tool", json!({"name": "list_tasks", "arguments": {}}))] {
        let r = asked(true, "tools/call", json!({"name": name, "arguments": args}));
        assert_eq!(r, json!({"content": [{"type": "text", "text": SAID}], "isError": true}), "{name}");
    }
}

#[test]
fn ac207_outside_a_pane_otherwise_the_session_still_pays_for_nothing() {
    assert!(asked(false, "initialize", json!({})).get("instructions").is_none());
    assert_eq!(asked(false, "tools/list", json!({}))["tools"], json!([]));
    let r = asked(false, "tools/call", json!({"name": "get_session_plan", "arguments": {}}));
    assert_eq!(r["content"][0]["text"], NOT_IN_PANE);
    let line = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": TOOL, "arguments": {}}}).to_string();
    let out: Value = serde_json::from_str(&answer(&line, &Env::default(), &no_call).unwrap()).unwrap();
    assert_eq!(out["error"]["code"], -32602, "no such tool outside a shared app-server: {out}");
}

#[test]
fn ac207_the_words_name_the_cause_and_every_fix() {
    for part in ["app-server", "codex -c tui.alternate_screen=never", "TMUX_PANE", "--no-daemon", "codex features disable daemon_auto_start", "codex app-server daemon stop"] {
        assert!(SAID.contains(part), "{part}");
    }
}
