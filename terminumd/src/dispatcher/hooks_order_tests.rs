// SPDX-License-Identifier: AGPL-3.0-only OR LicenseRef-Commercial
//! AC-157, AC-123, AC-138: hold the initial hook in the real queue until task tools have answered.

use super::*;
use crate::tasks::closeout::ASKED;
use crate::tasks::store::{Store, HUMAN};
use crate::tasks::tools::{self, Who};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const PROMPT: &str = "The human ran these tasks";
const RESULT: &str = "Outcome: Done\nDone: Fixed.\nChecked: Regression passed.\nNot done: None\nNeeds a human: None\nNew tasks: None\nTasks changed: None\nIssues and discoveries: None\nFurther recommendations: None";

struct Fixture {
    root: PathBuf,
    app: Shared,
    who: Who,
    run: String,
    ids: Vec<String>,
}

impl Fixture {
    fn new(statuses: &[&str]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let (root, mut app) = crate::testutil::app(&format!("hook-order-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
        Arc::get_mut(&mut app).unwrap().tasks = Arc::new(Store::open(root.join("kanban.sqlite")));
        let key = app.tmux.local.key("%1");
        let (run, ids) = app.tasks.with(|tx| {
            let mut tasks = Vec::new();
            for status in statuses {
                let t = ops::create(tx, HUMAN, ops::NewTask { title: "early work".into(), session: Some(key.clone()), ..Default::default() }, 1)?;
                ops::move_to(tx, HUMAN, &t.id, status, 2)?;
                tasks.push(t);
            }
            let like = Run { agent_id: "a".into(), agent_name: "Fixer".into(), kind: "claude".into(), project: "default".into(), prompt: Some(PROMPT.into()), ..Default::default() };
            let run = runs::add(tx, &like, &tasks.iter().map(|t| t.key.clone()).collect::<Vec<_>>(), Default::default(), 3)?;
            app.dispatcher.pane_opened(tx, &run, &key)?;
            runs::change(tx, &run, |r| r.state = Phase::Typed)?;
            Ok((run, tasks.into_iter().map(|t| t.id).collect()))
        }).unwrap();
        let who = Who { key, agent: "claude".into(), title: "Fixer".into(), project: Some("default".into()), dir: None, supervisor: false };
        Self { root, app, who, run, ids }
    }

    fn queue(&self, event: &str, prompt: &str) {
        heard(&self.app, &self.who.key, &json!({"event": event, "prompt": prompt, "session": "c0ffee"}), Whose::Main);
    }

    async fn drain(&self) {
        let mut events = self.app.dispatcher.events.waiting.lock().take().unwrap();
        while let Ok((key, ev)) = events.try_recv() {
            one_event(self.app.clone(), key, ev).await;
        }
        *self.app.dispatcher.events.waiting.lock() = Some(events);
    }

    fn tool(&self, tool: &str, args: Value) -> (String, Vec<String>) {
        tools::run(&self.app.tasks, &self.root, &self.who, tool, &args).unwrap()
    }

    fn read(&self) -> Run {
        self.app.tasks.with(|tx| runs::by_id(tx, &self.run)).unwrap()
    }

    fn statuses(&self) -> Vec<String> {
        self.app.tasks.with(|tx| self.ids.iter().map(|id| ops::get(tx, id).map(|t| t.status)).collect()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
    }
}

#[tokio::test]
async fn ac157_a_delayed_initial_hook_keeps_the_early_question_and_run_state() {
    let f = Fixture::new(&["TODO"]);
    f.queue("UserPromptSubmit", PROMPT);
    f.tool("ask_human", json!({"id": f.ids[0], "questions": ASKED}));
    let (_page, mut messages) = f.app.pages.add(Default::default(), String::new(), crate::page::trace::Recorder::new(f.root.clone())).unwrap();
    let history = f.app.tasks.with(|tx| ops::history(tx, &f.ids[0])).unwrap();
    f.app.tasks.with(|tx| runs::change(tx, &f.run, |r| { r.open_tools = 2; r.budget_used = 17; }).map(drop)).unwrap();
    f.drain().await;
    let r = f.read();
    assert_eq!(f.statuses(), ["WAIT FOR HUMAN"]);
    assert!(r.asked);
    assert_eq!((r.open_tools, r.budget_used), (2, 17), "the acknowledgment resets no work already done");
    assert_eq!(r.agent_session.as_deref(), Some("c0ffee"));
    assert!(r.prompt.is_none());
    assert_eq!(f.app.tasks.with(|tx| ops::history(tx, &f.ids[0])).unwrap(), history);
    let notices: Vec<Value> = std::iter::from_fn(|| messages.try_recv().ok()).map(|s| serde_json::from_str(&s).unwrap()).collect();
    assert!(notices.contains(&json!({"t": "taskchange", "id": f.ids[0]})), "the task window hears its newly known conversation: {notices:?}");
    f.queue("Stop", "");
    f.drain().await;
    assert!(f.read().turn_ended && f.read().asked);
}

#[tokio::test]
async fn ac123_a_delayed_initial_hook_keeps_early_results_and_moves() {
    for to in ["DONE", "FAILED"] {
        let f = Fixture::new(&["TODO", "TODO"]);
        f.queue("UserPromptSubmit", PROMPT);
        let (_, changed) = if to == "DONE" {
            f.tool("finish_task", json!({"id": f.ids[0], "result": RESULT}))
        } else {
            f.tool("move_task", json!({"id": f.ids[0], "to": to}))
        };
        assert!(changed.contains(&f.ids[1]), "every task started must be broadcast");
        f.drain().await;
        assert_eq!(f.statuses(), [to, "IN PROGRESS"]);
        if to == "DONE" {
            assert!(!f.read().result, "the second task has no result yet");
            f.tool("finish_task", json!({"id": f.ids[1], "result": RESULT}));
            assert!(f.read().result, "both real results stay recorded");
        }
    }
}

#[tokio::test]
async fn ac138_an_early_question_waits_with_open_siblings_and_keeps_finished_ones() {
    let f = Fixture::new(&["TODO", "TODO", "TODO"]);
    f.queue("UserPromptSubmit", PROMPT);
    f.tool("finish_task", json!({"id": f.ids[0], "result": RESULT}));
    f.tool("ask_human", json!({"id": f.ids[1], "questions": ASKED}));
    f.drain().await;
    assert_eq!(f.statuses(), ["DONE", "WAIT FOR HUMAN", "WAIT FOR HUMAN"]);
    assert!(f.read().asked);
}

#[tokio::test]
async fn ac157_normal_start_and_resumed_work_accept_the_initial_prompt_once() {
    for status in ["TODO", "WAIT FOR HUMAN", "DONE"] {
        let f = Fixture::new(&[status]);
        f.app.tasks.with(|tx| runs::change(tx, &f.run, |r| { r.asked = true; r.budget_used = 30; }).map(drop)).unwrap();
        f.queue("UserPromptSubmit", PROMPT);
        f.drain().await;
        assert_eq!(f.statuses(), ["IN PROGRESS"]);
        let r = f.read();
        assert!(!r.asked && !r.turn_ended && r.budget_used == 0);
        f.tool("ask_human", json!({"id": f.ids[0], "questions": ASKED}));
        // A human's real new prompt still starts new work after the first acknowledgment.
        f.queue("UserPromptSubmit", "Use v2");
        f.drain().await;
        assert_eq!(f.statuses(), ["IN PROGRESS"]);
        assert!(!f.read().asked);
    }
}

#[tokio::test]
async fn ac157_a_humans_new_prompt_cannot_consume_the_pending_first_acknowledgment() {
    let f = Fixture::new(&["WAIT FOR HUMAN"]);
    f.tool("ask_human", json!({"id": f.ids[0], "questions": format!("{ASKED} First.")}));
    // The new prompt overtakes the first hook in transport, rather than only in its queue.
    f.queue("UserPromptSubmit", "Use v2");
    f.drain().await;
    assert_eq!(f.statuses(), ["IN PROGRESS"]);
    f.tool("ask_human", json!({"id": f.ids[0], "questions": format!("{ASKED} Second.")}));
    f.queue("UserPromptSubmit", "The human\n ran these tasks");
    f.drain().await;
    assert_eq!(f.statuses(), ["WAIT FOR HUMAN"]);
    assert!(f.read().asked);
    assert_eq!(f.read().agent_session.as_deref(), Some("c0ffee"));
}

#[tokio::test]
async fn ac123_an_early_single_result_is_not_reopened_by_its_delayed_first_hook() {
    let f = Fixture::new(&["TODO"]);
    f.queue("UserPromptSubmit", PROMPT);
    f.tool("finish_task", json!({"id": f.ids[0], "result": RESULT}));
    f.drain().await;
    assert_eq!(f.statuses(), ["DONE"]);
    assert!(f.read().result);
}

#[tokio::test]
async fn ac157_a_refused_tool_does_not_accept_the_start() {
    let f = Fixture::new(&["TODO"]);
    let was = f.read();
    assert!(tools::run(&f.app.tasks, &f.root, &f.who, "ask_human", &json!({"id": f.ids[0]})).is_err());
    assert_eq!(f.read(), was, "the tool transaction rolls back its start too");
    assert_eq!(f.statuses(), ["TODO"]);
}

#[tokio::test]
async fn ac123_an_incomplete_result_still_broadcasts_every_task_started() {
    for result in ["no sections".to_string(), RESULT.replace("Outcome: Done", "Outcome: Not done")] {
        let f = Fixture::new(&["TODO", "TODO"]);
        let (_, changed) = f.tool("finish_task", json!({"id": f.ids[0], "result": result}));
        assert!(f.ids.iter().all(|id| changed.contains(id)));
        assert_eq!(f.statuses(), ["IN PROGRESS", "IN PROGRESS"]);
        assert!(!f.read().result);
    }
}

#[tokio::test]
async fn ac157_a_resumed_runs_early_question_or_result_survives_its_first_hook() {
    for status in ["WAIT FOR HUMAN", "DONE"] {
        for tool in ["ask_human", "finish_task"] {
            let f = Fixture::new(&[status]);
            f.app.tasks.with(|tx| runs::change(tx, &f.run, |r| { r.asked = true; r.budget_used = 30; }).map(drop)).unwrap();
            f.queue("UserPromptSubmit", PROMPT);
            f.tool(tool, json!({"id": f.ids[0], "questions": ASKED, "result": RESULT}));
            f.drain().await;
            assert_eq!(f.statuses(), [if tool == "ask_human" { "WAIT FOR HUMAN" } else { "DONE" }]);
            let r = f.read();
            assert_eq!(r.asked, tool == "ask_human");
            assert_eq!(r.result, tool == "finish_task");
            assert_eq!(r.budget_used, 0, "the resumed work starts its own clock");
        }
    }
}

#[tokio::test]
async fn ac157_a_taskless_session_is_not_accepted_by_a_task_tool() {
    let f = Fixture::new(&["TODO"]);
    f.app.tasks.with(|tx| {
        tx.execute("DELETE FROM run_tasks WHERE run = ?1", [&f.run]).map_err(ops::sql)?;
        tx.execute("UPDATE runs SET task = NULL, prompt = NULL WHERE id = ?1", [&f.run]).map(drop).map_err(ops::sql)
    }).unwrap();
    let (said, _) = f.tool("ask_human", json!({"id": f.ids[0], "questions": ASKED}));
    assert!(said.ends_with("End your turn now: the human answers on the board, and terminum types a notice with the task's ID into this session, or answers here."));
    assert_eq!(f.read().state, Phase::Typed);
    assert!(!f.read().asked);
}
