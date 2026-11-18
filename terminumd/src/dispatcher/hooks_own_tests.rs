// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-122, AC-90: a turn the agent scheduled for itself (Claude Code's ScheduleWakeup, CronCreate) is no one's
//! new work: through the real /hook and Dispatcher's queue, its task stays queued for integration.

use super::*;
use crate::tasks::store::{Store, HUMAN};
use serde_json::json;
use std::sync::Arc;

/// `evs` through the real /hook, then Dispatcher's queue taken in order.
async fn post(app: &Shared, server: &Arc<crate::tmux::server::Server>, events: &mut UnboundedReceiver<(String, Value)>, evs: Vec<Value>) {
    for e in evs {
        crate::sock::hook_on(app.clone(), Some(server.clone()), e).await;
    }
    while let Ok((k, e)) = events.try_recv() {
        one_event(app.clone(), k, e).await;
    }
}

#[tokio::test]
async fn ac122_a_wakeup_the_agent_scheduled_leaves_its_queued_task_in_integration() {
    let (root, mut app) = crate::testutil::app("hooks-wakeup");
    let file = root.join("scheduled-prompts.json");
    {
        let a = Arc::get_mut(&mut app).unwrap();
        a.tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
        a.hooks = crate::tmux::hookstate::Hooks::new(crate::messages::own::Scheduled::load(file.clone()));
    }
    let server = app.tmux.local.clone();
    server.set_panes(std::collections::HashMap::from([("%1".to_string(), "main".to_string())]));
    let key = server.key("%1");
    let (run, task) = app.tasks.with(|tx| {
        let t = crate::tasks::ops::create(tx, HUMAN, crate::tasks::ops::NewTask { title: "land it".into(), ..Default::default() }, 1)?;
        let like = runs::Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "default".into(), ..Default::default() };
        let id = runs::add(tx, &like, std::slice::from_ref(&t.key), Default::default(), 1)?;
        app.dispatcher.pane_opened(tx, &id, &key)?;
        runs::change(tx, &id, |r| r.state = Phase::Working)?;
        Ok((id, t.key))
    }).unwrap();
    let mut events = app.dispatcher.events.waiting.lock().take().unwrap();
    let ev = |name: &str, extra: Value| {
        let mut v = json!({"agent": "claude", "event": name, "pane": "%1", "session": "s1", "cwd": "/srv"});
        v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
        v
    };
    let wake = "Check check_changed.py for TERM-9, then land via make land SLUG=term-9 TASK=TERM-9 RESULT=file.";
    let history = || app.tasks.with(|tx| crate::tasks::comments::history(tx, &task)).unwrap();
    let status = || app.tasks.with(|tx| Ok(crate::tasks::ops::get(tx, &task)?.status)).unwrap();
    // the wakeup's tool came late, after its turn's Stop: it is remembered all the same
    post(&app, &server, &mut events, vec![
        ev("UserPromptSubmit", json!({"turn": "p1", "prompt": "queue the landing"})),
        ev("Stop", json!({"turn": "p1"})),
        ev("PreToolUse", json!({"turn": "p1", "tool": "ScheduleWakeup", "scheduled": wake})),
    ]).await;
    for _ in 0..300 {
        if crate::messages::own::Scheduled::load(file.clone()).has(&key, wake) { break }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(crate::messages::own::Scheduled::load(file.clone()).has(&key, wake), "kept for a terminumd started anew");
    // its turn queued the task (make land … TASK=…, terminum-integration.md AC-198, which this test does not run)
    app.tasks.with(|tx| {
        tx.execute("UPDATE tasks SET status = ?1 WHERE id = ?2", (crate::tasks::ops::INTEGRATION, &task)).map_err(crate::tasks::ops::sql)?;
        runs::change(tx, &run, |r| r.status = crate::tasks::ops::INTEGRATION.into())
    }).unwrap();
    let before = history();
    let human_at = std::time::Instant::now();
    post(&app, &server, &mut events, vec![ev("UserPromptSubmit", json!({"turn": "p2", "prompt": wake}))]).await;
    assert_eq!(status(), crate::tasks::ops::INTEGRATION, "the wakeup is no one's new work");
    assert_eq!(history(), before, "nothing moved it");
    let r = app.tasks.with(|tx| runs::by_id(tx, &run)).unwrap();
    assert!(!r.turn_ended, "its turn started all the same");
    assert!(!app.inputs.human_since(&key, human_at), "no human typed it (AC-131)");
    post(&app, &server, &mut events, vec![ev("Stop", json!({"turn": "p2"})), ev("UserPromptSubmit", json!({"turn": "p3", "prompt": "and fix the docs too"}))]).await;
    assert_eq!(status(), "IN PROGRESS", "a human's prompt is new work");
    drop(app);
    std::fs::remove_dir_all(root).unwrap();
}
