// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
use std::cell::RefCell;

use super::*;

const TMUX: &str = "/private/tmp/tmux-501/terminum,123,0";

fn in_pane() -> Env {
    Env { tmux: Some(TMUX.into()), pane: Some("%3".into()) }
}

fn no_call(_: &Value) -> Result<(u16, Option<Value>), Down> {
    panic!("nothing may be forwarded")
}

fn ask(line: &str) -> Value {
    serde_json::from_str(&answer(line, &in_pane(), &no_call).expect("an answer")).unwrap()
}

/// The result of tools/call `tool` with `args`, terminumd answering `reply`; and what it got.
fn tool(tool: &str, args: Value, env: &Env, reply: Result<(u16, Option<Value>), ()>) -> (Value, Option<Value>) {
    let sent = RefCell::new(None);
    let f = |b: &Value| {
        *sent.borrow_mut() = Some(b.clone());
        reply.clone().map_err(|_| Down)
    };
    let line = json!({"jsonrpc": "2.0", "id": 7, "method": "tools/call",
        "params": {"name": tool, "arguments": args, "_meta": {"sessionId": "019a-uuid"}}}).to_string();
    let out: Value = serde_json::from_str(&answer(&line, env, &f).unwrap()).unwrap();
    assert_eq!(out["id"], 7);
    (out["result"].clone(), sent.into_inner())
}

fn refused(t: &str, args: Value) -> String {
    let (r, sent) = tool(t, args, &in_pane(), Err(()));
    assert_eq!(r["isError"], true);
    assert!(sent.is_none(), "a refused call is not forwarded");
    r["content"][0]["text"].as_str().unwrap().to_string()
}

#[test]
fn initialize_answers_the_clients_version_when_known() {
    let r = ask(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{}}}"#);
    assert_eq!(r["id"], 0);
    assert_eq!(r["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(r["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(r["result"]["serverInfo"], json!({"name": "terminum", "version": crate::config::build_id()}));
    assert_eq!(r["result"]["instructions"], INSTRUCTIONS);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#)["result"]["protocolVersion"], "2025-03-26");
    for other in [r#""2026-07-28""#, r#""""#, "3"] {
        let line = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{"protocolVersion":{other}}}}}"#);
        assert_eq!(ask(&line)["result"]["protocolVersion"], "2025-06-18", "{other}");
    }
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#)["result"]["protocolVersion"], "2025-06-18");
}

#[test]
fn the_instructions_stand_in_512_characters() {
    let first: String = INSTRUCTIONS.chars().take(512).collect();
    assert_eq!(first, INSTRUCTIONS, "whole within 512, so a client's cut loses nothing");
    for w in ["update_session_plan", "eta_minutes", "add_notes_for_user", "every 10 minutes", "stop calling"] {
        assert!(first.contains(w), "{w}");
    }
    assert!(first.ends_with('.'));
}

#[test]
fn discover_and_unknown_methods_are_not_found_ping_is_empty() {
    let r = ask(r#"{"jsonrpc":"2.0","id":0,"method":"server/discover","params":{"_meta":{}}}"#);
    assert_eq!((r["id"].clone(), r["error"]["code"].clone()), (json!(0), json!(-32601)));
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":2,"method":"resources/list"}"#)["error"]["code"], -32601);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":3,"method":"ping"}"#), json!({"jsonrpc": "2.0", "id": 3, "result": {}}));
}

#[test]
fn notifications_responses_and_blank_lines_get_no_answer() {
    let env = in_pane();
    for line in [r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"get_session_plan"}}"#,
                 r#"{"jsonrpc":"2.0","method":"no/such"}"#, r#"{"jsonrpc":"2.0","id":5,"result":{}}"#, "", "   "] {
        assert_eq!(answer(line, &env, &no_call), None, "{line}");
    }
}

#[test]
fn garbage_is_a_parse_error_never_a_panic() {
    for line in ["{nope", "\u{0}line too long", "{\"id\":1", "\u{fffd}\u{fffd}"] {
        let r = ask(line);
        assert_eq!((r["id"].clone(), r["error"]["code"].clone()), (Value::Null, json!(-32700)), "{line}");
    }
    assert_eq!(ask("[1,2]")["error"]["code"], -32600, "no batches");
    assert_eq!(ask("42")["error"]["code"], -32600);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1}"#)["error"]["code"], -32600, "no method");
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":{"a":1},"method":"ping"}"#)["error"]["code"], -32600);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"rm_rf"}}"#)["error"]["code"], -32602);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":7}"#)["error"]["code"], -32602);
}

#[test]
fn a_line_too_long_is_dropped_to_its_newline() {
    let mut input = vec![b'x'; MAX_LINE + 10];
    input.extend_from_slice(b"\n{\"id\":1}\r\nlast");
    let mut r = std::io::Cursor::new(input);
    assert_eq!(read_line(&mut r).as_deref(), Some("\u{0}line too long"));
    assert_eq!(read_line(&mut r).as_deref(), Some("{\"id\":1}"));
    assert_eq!(read_line(&mut r).as_deref(), Some("last"));
    assert_eq!(read_line(&mut r), None);
}

#[test]
fn tools_list_has_four_tools_with_schemas_and_hints() {
    let r = ask(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{"_meta":{"progressToken":1}}}"#);
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, TOOLS);
    for t in tools {
        assert_eq!(t["annotations"], json!({"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}));
        assert_eq!(t["inputSchema"]["type"], "object");
        assert!(t["description"].as_str().unwrap().len() < 500);
    }
    let plan = &tools[0]["inputSchema"];
    assert_eq!(plan["required"], json!(["items"]));
    assert_eq!((plan["properties"]["items"]["minItems"].clone(), plan["properties"]["items"]["maxItems"].clone()), (json!(1), json!(50)));
    assert_eq!(plan["properties"]["items"]["items"]["properties"]["status"]["enum"], json!(["pending", "in_progress", "completed", "blocked"]));
    assert_eq!(plan["properties"]["items"]["items"]["properties"]["text"]["maxLength"], 200);
    assert_eq!((plan["properties"]["eta_minutes"]["minimum"].clone(), plan["properties"]["eta_minutes"]["maximum"].clone()), (json!(1), json!(1440)));
    assert_eq!((plan["properties"]["doing"]["maxLength"].clone(), plan["properties"]["title"]["maxLength"].clone()), (json!(120), json!(80)));
    let d = tools[0]["description"].as_str().unwrap();
    assert!(d.contains("whole plan") && d.contains("pending, in_progress, completed or blocked") && d.contains("minutes"));
    let notes = &tools[2]["inputSchema"]["properties"]["notes"];
    assert_eq!((notes["maxItems"].clone(), notes["items"]["properties"]["kind"]["enum"].clone()), (json!(20), json!(["bug", "deferred", "followup", "question"])));
    assert_eq!((notes["items"]["properties"]["text"]["maxLength"].clone(), notes["items"]["properties"]["file"]["maxLength"].clone()), (json!(1000), json!(300)));
    assert_eq!(tools[3]["inputSchema"]["required"], json!(["id"]));
    assert_eq!(tools[3]["inputSchema"]["properties"]["status"]["enum"], json!(["open", "done"]));
}

#[test]
fn a_call_goes_to_terminumd_with_the_pane_and_only_known_fields() {
    let args = json!({"items": [{"text": "Write the parser", "status": "completed", "extra": 1}, {"text": "Add tests", "status": "in_progress"}],
        "doing": "Run the tests", "eta_minutes": 45, "title": null, "percent": 90});
    let reply = Ok((200, Some(json!({"text": "Plan stored: 1 of 2 completed (50%), ETA ~17:40.", "error": false}))));
    let (r, sent) = tool("update_session_plan", args, &in_pane(), reply);
    assert_eq!(r, json!({"content": [{"type": "text", "text": "Plan stored: 1 of 2 completed (50%), ETA ~17:40."}], "isError": false}));
    assert_eq!(sent.unwrap(), json!({"socket": "terminum", "pane": "%3", "tool": "update_session_plan", "session": "019a-uuid",
        "args": {"items": [{"text": "Write the parser", "status": "completed"}, {"text": "Add tests", "status": "in_progress"}],
                 "doing": "Run the tests", "eta_minutes": 45}}));
    let (r, sent) = tool("get_session_plan", json!({}), &in_pane(), Ok((200, Some(json!({"text": "[x] a", "error": true})))));
    assert_eq!((r["isError"].clone(), sent.unwrap()["args"].clone()), (json!(true), json!({})), "terminumd's error flag kept");
    let (_, sent) = tool("add_notes_for_user", json!({"notes": [{"kind": "bug", "text": "The parser drops a BOM", "file": "src/parse.rs"}]}), &in_pane(), Ok((200, Some(json!({"text": "n1"})))));
    assert_eq!(sent.unwrap()["args"], json!({"notes": [{"kind": "bug", "text": "The parser drops a BOM", "file": "src/parse.rs"}]}));
    let (_, sent) = tool("update_notes_for_user", json!({"id": "n1", "status": "done"}), &in_pane(), Ok((200, Some(json!({"text": "ok"})))));
    assert_eq!(sent.unwrap()["args"], json!({"id": "n1", "status": "done"}));
}

#[test]
fn without_a_session_id_none_is_sent() {
    let sent = RefCell::new(None);
    let f = |b: &Value| { *sent.borrow_mut() = Some(b.clone()); Ok((200, Some(json!({"text": "ok"})))) };
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"get_session_plan","_meta":{"claudecode/toolUseId":"toolu_1"}}}"#;
    answer(line, &in_pane(), &f).unwrap();
    assert!(sent.into_inner().unwrap().get("session").is_none(), "Claude Code gives no session id");
}

#[test]
fn outside_a_pane_each_tool_says_so_and_nothing_is_sent() {
    let outside = [Env { tmux: None, pane: None }, Env { tmux: Some(TMUX.into()), pane: None }, Env { tmux: None, pane: Some("%3".into()) },
                   Env { tmux: Some(TMUX.into()), pane: Some("%x".into()) }, Env { tmux: Some(TMUX.into()), pane: Some("%".into()) },
                   Env { tmux: Some(",1,0".into()), pane: Some("%3".into()) }];
    for env in &outside {
        for t in TOOLS {
            let args = match t {
                "update_session_plan" => json!({"items": [{"text": "a", "status": "pending"}]}),
                "add_notes_for_user" => json!({"notes": [{"kind": "bug", "text": "a"}]}),
                "update_notes_for_user" => json!({"id": 1, "status": "done"}),
                _ => json!({}),
            };
            let (r, sent) = tool(t, args, env, Err(()));
            assert_eq!(r, json!({"content": [{"type": "text", "text": NOT_IN_PANE}], "isError": true}), "{t}");
            assert!(sent.is_none());
        }
    }
}

#[test]
fn terminumd_down_or_refusing_is_an_error_result() {
    let (r, sent) = tool("get_session_plan", json!({}), &in_pane(), Err(()));
    assert_eq!((r["content"][0]["text"].clone(), r["isError"].clone()), (json!(DOWN), json!(true)));
    assert!(sent.is_some());
    let (r, _) = tool("get_session_plan", json!({}), &in_pane(), Ok((403, Some(json!({"error": "forbidden", "message": "not a pane of terminum"})))));
    assert_eq!(r["content"][0]["text"], "terminum refused the call: not a pane of terminum");
    let (r, _) = tool("get_session_plan", json!({}), &in_pane(), Ok((404, None)));
    assert_eq!((r["content"][0]["text"].clone(), r["isError"].clone()), (json!("terminum refused the call: HTTP 404"), json!(true)));
    let (r, _) = tool("get_session_plan", json!({}), &in_pane(), Ok((200, Some(json!({"ok": true})))));
    assert_eq!(r["isError"], true, "a 200 without text is no answer");
}

#[test]
fn update_session_plan_is_checked_before_it_goes() {
    let item = |t: &str, s: &str| json!({"text": t, "status": s});
    assert!(refused("update_session_plan", json!({})).contains("send the whole plan"));
    assert!(refused("update_session_plan", json!({"items": []})).contains("send the whole plan"));
    assert!(refused("update_session_plan", json!({"items": vec![item("a", "pending"); 51]})).contains("at most 50"));
    let long = "x".repeat(201);
    assert!(refused("update_session_plan", json!({"items": [item(&long, "pending")]})).contains("items[0].text is over 200"));
    let bad = refused("update_session_plan", json!({"items": [item("a", "pending"), item("b", "done")]}));
    assert!(bad.contains("items[1].status") && bad.contains("pending, in_progress, completed, blocked"), "{bad}");
    assert!(refused("update_session_plan", json!({"items": [{"text": "a"}]})).contains("status is required: one of pending"));
    assert!(refused("update_session_plan", json!({"items": [item(" ", "pending")]})).contains("items[0].text is empty"));
    assert!(refused("update_session_plan", json!({"items": ["a"]})).contains("items[0] must be an object"));
    for eta in [json!(0), json!(1441), json!(-5), json!(2.5), json!(30.0), json!("45")] {
        assert!(refused("update_session_plan", json!({"items": [item("a", "pending")], "eta_minutes": eta})).contains("from 1 to 1440"), "{eta}");
    }
    assert!(refused("update_session_plan", json!({"items": [item("a", "pending")], "doing": "d".repeat(121)})).contains("doing is over 120"));
    assert!(refused("update_session_plan", json!({"items": [item("a", "pending")], "title": "t".repeat(81)})).contains("title is over 80"));
    assert!(refused("update_session_plan", json!([1])).contains("must be an object"));
    // the edges kept: 50 items, 200 characters (not bytes), 1 and 1440 minutes (integers only, as terminumd takes them)
    for (n, eta) in [(50, json!(1)), (1, json!(1440))] {
        let args = json!({"items": vec![item(&"é".repeat(200), "blocked"); n], "eta_minutes": eta});
        assert!(checked("update_session_plan", &args).is_ok(), "{n} {eta}");
    }
}

#[test]
fn notes_are_checked_before_they_go() {
    let note = |k: &str, t: &str| json!({"kind": k, "text": t});
    assert!(refused("add_notes_for_user", json!({"notes": []})).contains("1–20 notes"));
    assert!(refused("add_notes_for_user", json!({"notes": vec![note("bug", "a"); 21]})).contains("at most 20"));
    let k = refused("add_notes_for_user", json!({"notes": [note("todo", "a")]}));
    assert!(k.contains("notes[0].kind") && k.contains("bug, deferred, followup, question"), "{k}");
    assert!(refused("add_notes_for_user", json!({"notes": [note("bug", &"x".repeat(1001))]})).contains("over 1000"));
    assert!(refused("add_notes_for_user", json!({"notes": [{"kind": "bug", "text": "a", "file": "f".repeat(301)}]})).contains("file is over 300"));
    assert!(refused("add_notes_for_user", json!({"notes": [{"kind": "bug"}]})).contains("text is required"));
    assert!(refused("update_notes_for_user", json!({"status": "done"})).contains("id is required"));
    assert!(refused("update_notes_for_user", json!({"id": -1, "status": "done"})).contains("id is required"));
    assert!(refused("update_notes_for_user", json!({"id": "n1"})).contains("nothing to change"));
    assert!(refused("update_notes_for_user", json!({"id": "n1", "status": "closed"})).contains("open, done"));
    assert!(refused("update_notes_for_user", json!({"id": "n1", "kind": "idea"})).contains("bug, deferred, followup, question"));
    assert!(checked("add_notes_for_user", &json!({"notes": vec![note("question", "a"); 20]})).is_ok());
    assert_eq!(checked("update_notes_for_user", &json!({"id": 4, "text": "new", "kind": "followup"})).unwrap(), json!({"id": 4, "text": "new", "kind": "followup"}));
}

#[test]
fn mcp_is_a_command_without_arguments() {
    let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(crate::cli::parse(&args(&["mcp"])), Ok(crate::cli::Cmd::Mcp));
    assert!(crate::cli::parse(&args(&["mcp", "x"])).is_err());
    assert!(crate::cli::wanted(&args(&["/Users/alex/.terminum/bin/terminumd", "mcp"])));
}

#[test]
fn outside_tmux_initialize_has_no_instructions_and_no_tools() {
    for env in [Env { tmux: None, pane: None }, Env { tmux: Some(TMUX.into()), pane: None }, Env { tmux: None, pane: Some("%3".into()) }] {
        let say = |line: &str| -> Value { serde_json::from_str(&answer(line, &env, &no_call).unwrap()).unwrap() };
        let init = say(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#);
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(init["result"].get("instructions").is_none(), "{init}");
        assert_eq!(say(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)["result"], json!({"tools": []}));
    }
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":0,"method":"initialize"}"#)["result"]["instructions"], INSTRUCTIONS, "in a pane: kept");
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#)["result"]["tools"].as_array().unwrap().len(), 4);
}

#[test]
fn lengths_count_after_cleaning_and_eta_is_an_integer() {
    let padded = format!("{}{}", "x".repeat(200), "\u{202E}\u{7}".repeat(20));
    let (r, sent) = tool("update_session_plan", json!({"items": [{"text": padded, "status": "pending"}]}), &in_pane(), Err(()));
    assert_eq!(sent.unwrap()["args"]["items"][0]["text"], "x".repeat(200), "{r}");
    assert!(refused("update_session_plan", json!({"items": [{"text": "\u{2066}\u{7}", "status": "pending"}]})).contains("empty"));
    assert!(refused("update_session_plan", json!({"items": [{"text": "a", "status": "pending"}], "eta_minutes": 30.0})).contains("whole number"));
    assert!(refused("add_notes_for_user", json!({"notes": [{"kind": "bug", "text": "x".repeat(1001)}]})).contains("over 1000"));
}
