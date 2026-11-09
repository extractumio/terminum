// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! What an agent's hook event means to its run (`terminum-dispatcher.md` AC-121 to AC-124): acted on
//! at once, written when it changes. A hook's fields are data: any process of the user can post one,
//! so only a shape-checked ID is kept, and its words are cleaned.

use std::future::Future;

use parking_lot::Mutex;
use serde_json::Value;
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use super::runs::{self, Phase, Run};
use super::{move_all, move_one, move_task, store};
use crate::app::Shared;
use crate::plans::rules;
use crate::tasks::ops;
use crate::tmux::hookstate::Whose;

/// The shape a session ID must have to reach a command line (§9 Resume): no flag.
pub fn good_id(id: &str) -> bool {
    id.len() <= 100 && id.starts_with(|c: char| c.is_ascii_alphanumeric()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// What the event changes in `r`: whether it moved the task, whether it was activity. `typed`: this
/// prompt is what terminum typed.
pub(super) fn apply(tx: &rusqlite::Transaction, r: &mut Run, ev: &Value, typed: bool) -> Result<(bool, bool), String> {
    let event = ev["event"].as_str().unwrap_or("");
    let session = ev["session"].as_str().filter(|s| good_id(s));
    let mut moved = false;
    let mut active = !r.nudge_turn;                    // the turn our request started is no new work (AC-123)
    match event {
        "SessionStart" => {
            r.agent_session = session.map(str::to_string).or(r.agent_session.take());
            active = false;
        }
        "UserPromptSubmit" if r.is_starting() => {
            r.state = Phase::Working;
            r.prompt = None;
            r.new_work();
            r.turn_starts();
            r.agent_session = session.map(str::to_string).or(r.agent_session.take());
            let agent = ops::pool_user(tx, &r.agent_id, &r.agent_name)?;
            for id in &r.ids {
                ops::set_owner(tx, &ops::key_of(tx, id)?, &agent, ops::now())?;
            }
            move_all(tx, r, "IN PROGRESS", "The agent took it.")?;
            moved = true;
        }
        "UserPromptSubmit" if typed => active = false,
        "UserPromptSubmit" => {
            // a human's own prompt: new work (AC-122); to which of several finished tasks, it does not say
            active = true;
            r.new_work();
            r.turn_starts();
            if r.status != "IN PROGRESS" && (r.ids.len() == 1 || !r.open.is_empty()) {
                for id in r.targets() {
                    move_one(tx, id, "IN PROGRESS", "Dispatcher moved it: new work in its session")?;
                }
                moved = true;
            }
        }
        "Stop" => {
            r.turn_over();
            r.nudge_turn = false;
        }
        "StopFailure" => {
            r.turn_over();
            active = true;
            if !super::plan::finished(&r.status) {
                let why: String = rules::clean(ev["error"].as_str().unwrap_or("an error")).chars().take(60).collect();
                move_task(tx, r, "FAILED", &format!("The agent's turn failed: {why}"))?;
                moved = true;
            }
        }
        "PreToolUse" => r.open_tools += 1,
        "PostToolUse" | "PostToolUseFailure" => r.open_tools = (r.open_tools - 1).max(0),
        "SessionEnd" => active = false,
        _ => {}
    }
    Ok((moved, active))
}

/// Hooks' events waiting for their turn: what comes first is applied first.
pub(super) struct Events {
    pub(super) sender: UnboundedSender<(String, Value)>,
    waiting: Mutex<Option<UnboundedReceiver<(String, Value)>>>,
}

impl Default for Events {
    fn default() -> Events {
        let (sender, waiting) = unbounded_channel();
        Events { sender, waiting: Mutex::new(Some(waiting)) }
    }
}

/// Pane `key`'s agent spoke: its run, if Dispatcher started it. A pane with none costs a lookup. Its subagents are
/// counted from their own events; only the main agent's events of its current turn change the run (AC-127).
pub fn heard(app: &Shared, key: &str, ev: &Value, whose: Whose) {
    if !app.dispatcher.has_run(key) {
        return;
    }
    app.dispatcher.heard_facts(key, ev);
    match whose {
        Whose::Main => drop(app.dispatcher.events.sender.send((key.to_string(), ev.clone()))),
        Whose::Sub => app.dispatcher.touched(key),     // activity, and no more
        Whose::Stale => {}
    }
}

/// What one of the main agent's events does to its run.
async fn one_event(app: Shared, key: String, ev: Value) {
    if ev["event"] == "Tokens" {
        let Some(models) = super::usage::by_model(&ev["tokens"]) else { return };
        let done = store(&app, move |tx| {
            let Some(r) = runs::in_pane(tx, &key)? else { return Ok(()) };
            super::usage::heard(tx, &r.id, &r.agent_id, &models, ops::now())
        }).await;
        if let Err(e) = done {
            tracing::warn!(event = "dispatcher.usage", error = %e);
        }
        return;
    }
    let typed = ev["event"] == "UserPromptSubmit" && ev["prompt"].as_str().is_some_and(|p| app.dispatcher.was_typed(&key, p));
    let ended = ev["event"] == "SessionEnd" && ev["reason"] != "clear";
    let k = key.clone();
    let done = store(&app, move |tx| {
        let Some(mut r) = runs::in_pane(tx, &k)? else { return Ok(None) };
        let was = r.clone();
        let (moved, active) = apply(tx, &mut r, &ev, typed)?;
        if r != was {
            runs::save(tx, &r)?;
        }
        Ok(Some((r, moved, active)))
    }).await;
    match done {
        Ok(Some((r, moved, active))) => {
            if active {
                app.dispatcher.touched(&key);
            }
            if moved {
                super::told_all(&app, &r);
            }
        }
        Ok(None) => return,
        Err(e) => tracing::warn!(event = "dispatcher.hook", error = %e),
    }
    // an end: the exit code decides, at a walk now (AS-127)
    if ended {
        app.dispatcher.soon.notify_one();
    }
}

/// Each item in its turn; one that panics goes to `panicked` and takes no other with it.
async fn in_turn<T, F: Future<Output = ()> + Send + 'static>(mut items: UnboundedReceiver<T>, work: impl Fn(T) -> F, panicked: impl Fn(tokio::task::JoinError)) {
    while let Some(item) = items.recv().await {
        if let Err(e) = tokio::spawn(work(item)).await {
            panicked(e);
        }
    }
}

/// Hooks' events, one at a time (AC-131): a Stop never overtakes the prompt before it.
pub async fn hearing(app: Shared) {
    let Some(events) = app.dispatcher.events.waiting.lock().take() else { return };
    let told = app.clone();
    in_turn(events, move |(key, ev)| one_event(app.clone(), key, ev), move |e| {
        tracing::error!(event = "dispatcher.panic", error = %e);
        super::acts::note(&told, "Dispatcher could not take an agent's event: see terminumd's log. It goes on with the next.".to_string());
    }).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::store::{Store, HUMAN};
    use std::sync::Arc;
    use std::time::Duration;

    #[tokio::test]
    async fn ac131_a_slow_event_is_not_overtaken_by_the_next() {
        let (sender, items) = unbounded_channel();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        for item in [("Prompt", 40), ("Stop", 0)] {
            sender.send(item).unwrap();
        }
        drop(sender);
        in_turn(items, move |(name, wait)| {
            let log = log.clone();
            async move {
                tokio::time::sleep(Duration::from_millis(wait)).await;
                log.lock().push(name);
            }
        }, |e| panic!("{e}")).await;
        assert_eq!(*seen.lock(), vec!["Prompt", "Stop"]);
    }

    #[tokio::test]
    async fn ac131_a_panic_is_said_and_takes_only_its_own_event() {
        let (sender, items) = unbounded_channel();
        let (seen, said) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(0)));
        let (log, count) = (seen.clone(), said.clone());
        for item in [true, false] {
            sender.send(item).unwrap();
        }
        drop(sender);
        in_turn(items, move |bad| {
            let log = log.clone();
            async move {
                assert!(!bad, "a test's own panic");
                log.lock().push(bad);
            }
        }, move |_| *count.lock() += 1).await;
        assert_eq!(*seen.lock(), vec![false]);
        assert_eq!(*said.lock(), 1);
    }

    /// TERM-34, across the hooks' record, Dispatcher and the plans: a subagent's failure and prompt, a late tool and a
    /// late Stop of an older turn leave the main run, its task and its plan as the main agent left them.
    #[tokio::test]
    async fn ac127_only_the_main_agents_events_of_its_turn_move_its_run_task_and_plan() {
        let (root, mut app) = crate::testutil::app("hooks-whose");
        std::sync::Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
        let server = app.tmux.local.clone();
        server.set_panes(std::collections::HashMap::from([("%1".to_string(), "main".to_string())]));
        let key = server.key("%1");
        let (run, task) = app.tasks.with(|tx| {
            let t = crate::tasks::ops::create(tx, HUMAN, crate::tasks::ops::NewTask { title: "one".into(), ..Default::default() }, 1)?;
            let like = runs::Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "default".into(), ..Default::default() };
            let id = runs::add(tx, &like, std::slice::from_ref(&t.key), 1)?;
            app.dispatcher.pane_opened(tx, &id, &key)?;
            runs::change(tx, &id, |r| r.state = Phase::Working)?;
            Ok((id, t.key))
        }).unwrap();
        let mut events = app.dispatcher.events.waiting.lock().take().unwrap();
        let ev = |name: &str, extra: Value| {
            let mut v = serde_json::json!({"agent": "claude", "event": name, "pane": "%1", "session": "s1"});
            v.as_object_mut().unwrap().extend(extra.as_object().cloned().unwrap_or_default());
            v
        };
        let done = serde_json::json!({"op": "all", "items": [{"text": "the main plan", "status": "completed"}]});
        for e in [
            ev("UserPromptSubmit", serde_json::json!({"turn": "p1", "prompt": "fix it", "cwd": "/srv"})),
            ev("PostToolUse", serde_json::json!({"turn": "p1", "tool": "TodoWrite", "plan": done})),
            ev("SubagentStart", serde_json::json!({"turn": "p1", "sub": "a1"})),
            ev("UserPromptSubmit", serde_json::json!({"turn": "p1", "sub": "a1", "prompt": "the child's own", "cwd": "/srv"})),
            ev("StopFailure", serde_json::json!({"turn": "p1", "sub": "a1", "error": "rate_limit"})),
        ] {
            crate::sock::hook_on(app.clone(), Some(server.clone()), e).await;
        }
        while let Ok((k, e)) = events.try_recv() {
            one_event(app.clone(), k, e).await;
        }
        let read = || app.tasks.with(|tx| Ok((runs::by_id(tx, &run)?, crate::tasks::ops::get(tx, &task)?.status))).unwrap();
        let (r, status) = read();
        assert_eq!(status, "IN PROGRESS", "the child's failure is its own");
        assert!(!r.turn_ended);
        assert_eq!(app.plans.details(&key, false, 2).map(|d| d.items.len()), Some(1), "the child's prompt starts no turn: the finished plan stays");
        for e in [
            ev("Stop", serde_json::json!({"turn": "p1"})),
            ev("PreToolUse", serde_json::json!({"turn": "p1", "tool": "Bash: ls"})),
            ev("UserPromptSubmit", serde_json::json!({"turn": "p2", "prompt": "and now this", "cwd": "/srv"})),
            ev("Stop", serde_json::json!({"turn": "p1"})),
        ] {
            crate::sock::hook_on(app.clone(), Some(server.clone()), e).await;
        }
        while let Ok((k, e)) = events.try_recv() {
            one_event(app.clone(), k, e).await;
        }
        let (r, status) = read();
        assert_eq!((r.turn_ended, r.open_tools, status.as_str()), (false, 0, "IN PROGRESS"), "p2 goes on: the late tool and Stop of p1 are no part of it");
        assert!(app.hooks.working(&key));
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ac131_a_walk_before_or_after_a_window_opens_keeps_its_pane() {
        let store = Store::open(crate::testutil::fresh_dir("hooks-panes").join("kanban.sqlite"));
        let d = crate::dispatcher::Dispatcher::default();
        let run = store.with(|tx| {
            let t = crate::tasks::ops::create(tx, HUMAN, crate::tasks::ops::NewTask { title: "one".into(), ..Default::default() }, 1)?;
            let like = runs::Run { agent_id: "a".into(), kind: "claude".into(), command: "claude".into(), folder: "/srv".into(), project: "default".into(), ..Default::default() };
            runs::add(tx, &like, std::slice::from_ref(&t.key), 1)
        }).unwrap();
        store.with(|tx| d.read_runs(tx)).unwrap();
        assert!(!d.has_run("%7"), "no pane yet");
        store.with(|tx| d.pane_opened(tx, &run, "%7")).unwrap();
        assert!(d.has_run("%7"), "a walk that read first has come and gone");
        store.with(|tx| d.read_runs(tx)).unwrap();
        assert!(d.has_run("%7"), "a walk that reads after it sees it in the store");
        store.with(|tx| runs::change(tx, &run, |r| r.state = Phase::Closed).map(drop)).unwrap();
        store.with(|tx| d.read_runs(tx)).unwrap();
        assert!(!d.has_run("%7"), "over: its hooks cost nothing again");
    }
}
