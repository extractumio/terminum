// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! The socket's /hook: whose an event is, what it changes, what its answer asks of the hook.

use super::*;

#[test]
fn hook_dispatch_waits_before_accepting_identity_or_mutating_plans() {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let _entered = rt.enter();
    let (root, app) = crate::testutil::app("hook-dispatch-order");
    let server = app.tmux.local.clone();
    server.set_panes(std::collections::HashMap::from([("%1".to_string(), "main".to_string())]));
    let key = server.key("%1");
    let serial = app.hooks.dispatch.lock();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let a = app.clone();
    let worker = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        started_tx.send(()).unwrap();
        let response = rt.block_on(hook_on(a, Some(server), json!({"agent": "claude", "event": "UserPromptSubmit",
            "pane": "%1", "session": "new", "plan": {"op": "all", "items": [{"text": "current"}]}})));
        done_tx.send(response.status()).unwrap();
    });
    started_rx.recv().unwrap();
    let blocked = done_rx.recv_timeout(std::time::Duration::from_millis(50)).is_err();
    let untouched = app.plans.details(&key, false, 2).is_none() && !app.hooks.working(&key);
    drop(serial);
    worker.join().unwrap();
    assert!(blocked && untouched, "hook acceptance and side effects bypassed dispatch serialization");
    assert_eq!(app.plans.details(&key, false, 2).unwrap().items[0].text, "current");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

/// AC-153: the hook is asked for the agent's command line at a start or a prompt until one came, then never.
#[tokio::test]
async fn ac153_the_command_line_is_asked_for_once_a_session() {
    let (root, app) = crate::testutil::app("hook-argv");
    let server = app.tmux.local.clone();
    server.set_panes(std::collections::HashMap::from([("%1".to_string(), "main".to_string())]));
    let key = server.key("%1");
    let post = |v: serde_json::Value| hook_on(app.clone(), Some(server.clone()), v);
    let asked = |r: Response| async move {
        let body = axum::body::to_bytes(r.into_body(), 1 << 16).await.unwrap();
        serde_json::from_slice::<serde_json::Value>(&body).ok().is_some_and(|v| v["argv"] == true)
    };
    let ev = |event: &str, extra: serde_json::Value| {
        let mut v = json!({"agent": "claude", "event": event, "pane": "%1", "session": "s1", "cwd": "/Users/alex/api"});
        v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        v
    };
    assert!(asked(post(ev("SessionStart", json!({}))).await).await, "a start asks");
    assert!(!asked(post(ev("PreToolUse", json!({"tool": "Bash"}))).await).await, "a tool's event never does");
    assert!(!asked(post(ev("UserPromptSubmit", json!({"sub": "a1", "prompt": "x"}))).await).await, "nor a subagent's prompt");
    assert_eq!(post(ev("Argv", json!({"session": "other", "argv": "claude --theirs"}))).await.status(), StatusCode::NO_CONTENT);
    assert!(asked(post(ev("UserPromptSubmit", json!({"prompt": "go"}))).await).await, "another session's line is not its: asked again");
    post(ev("Argv", json!({"argv": "claude --model opus"}))).await;
    assert!(!asked(post(ev("UserPromptSubmit", json!({"prompt": "on"}))).await).await, "once it came, never again");
    assert_eq!(app.hooks.started(&key).unwrap().argv, "claude --model opus");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn rejected_session_events_have_no_plan_or_turn_side_effects() {
    let (root, app) = crate::testutil::app("stale-hook-side-effects");
    let server = app.tmux.local.clone();
    server.set_panes(std::collections::HashMap::from([("%1".to_string(), "main".to_string())]));
    let key = server.key("%1");
    app.hooks.apply_event(&key, &json!({"agent": "claude", "event": "UserPromptSubmit", "session": "new"}), std::time::Instant::now());
    app.plans.hook(&key, "claude", Some("new"), &json!({"op": "all", "items": [{"text": "current"}]}), 1);
    for event in ["PostToolUse", "SessionEnd"] {
        let response = hook_on(app.clone(), Some(server.clone()), json!({"agent": "claude", "event": event,
            "pane": "%1", "session": "old", "plan": {"op": "all", "items": [{"text": "stale"}]}})).await;
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
        assert_eq!(app.plans.details(&key, false, 2).unwrap().items[0].text, "current");
        assert!(app.hooks.working(&key));
        assert!(!app.hooks.take_ended(&key));
    }
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}

#[tokio::test]
async fn ac90_an_agents_own_turns_and_a_subagents_prompts_are_no_ones_messages() {
    let (root, mut app) = crate::testutil::app("hook-injected");
    std::sync::Arc::get_mut(&mut app).unwrap().tasks = std::sync::Arc::new(crate::tasks::store::Store::open(root.join("kanban.sqlite")));
    let server = app.tmux.local.clone();
    server.set_panes(std::collections::HashMap::from([("%1".to_string(), "main".to_string())]));
    let key = server.key("%1");
    let human = || app.inputs.human_since(&key, std::time::Instant::now() - std::time::Duration::from_secs(60));
    let prompt = |p: &str, sub: Option<&str>| json!({"agent": "claude", "event": "UserPromptSubmit", "pane": "%1", "session": "s1",
        "cwd": "/Users/alex/api", "prompt": p, "sub": sub});
    for (p, sub) in [("<task-notification>\n<task-id>b1</task-id>", None), ("the child's own", Some("a1"))] {
        hook_on(app.clone(), Some(server.clone()), prompt(p, sub)).await;
        assert!(!human(), "no human typed {p}");
    }
    hook_on(app.clone(), Some(server.clone()), prompt("fix the <task-notification> parser", None)).await;
    assert!(human());
    let kept = || app.tasks.with(|tx| crate::messages::nth_of_folder(tx, "local", "/Users/alex/api", 0)).unwrap();
    for _ in 0..300 {
        if kept().0 > 0 { break }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (n, last) = kept();
    assert_eq!((n, last.unwrap()["text"].as_str().map(str::to_string)), (1, Some("fix the <task-notification> parser".into())));
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
